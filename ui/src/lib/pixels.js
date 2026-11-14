// Pixels for thumbnails: one frame fetched as Explicit VR Little Endian and windowed onto a canvas.

import { HttpError, WEB } from "@/lib/api";
import { authFetch } from "@/lib/session";

export const tag = (meta, t) => meta[t]?.Value;
export const first = (meta, t, fallback) => {
  const v = tag(meta, t)?.[0];
  return v == null || v === "" ? fallback : v;
};
export const num = (meta, t, fallback) => {
  const v = Number(first(meta, t, NaN));
  return isFinite(v) ? v : fallback;
};

export function imageInfo(meta) {
  const center = num(meta, "00281050", NaN);
  const width = num(meta, "00281051", NaN);
  return {
    rows: num(meta, "00280010", 0),
    columns: num(meta, "00280011", 0),
    bitsAllocated: num(meta, "00280100", 16),
    signed: num(meta, "00280103", 0) === 1,
    samples: num(meta, "00280002", 1),
    planar: num(meta, "00280006", 0),
    photometric: String(first(meta, "00280004", "MONOCHROME2")),
    slope: num(meta, "00281053", 1),
    intercept: num(meta, "00281052", 0),
    frames: num(meta, "00280008", 1),
    spacing: (tag(meta, "00280030") || [1, 1]).map(Number),
    window: isFinite(center) && width > 0 ? { center, width } : null,
  };
}

// the first part of a multipart/related answer
export function firstPart(buffer, contentType) {
  const boundary = /boundary="?([^";]+)"?/i.exec(contentType)?.[1];
  const data = new Uint8Array(buffer);
  if (!boundary) return data;
  const find = (needle, from) => {
    outer: for (let i = from; i <= data.length - needle.length; i++) {
      for (let j = 0; j < needle.length; j++) if (data[i + j] !== needle[j]) continue outer;
      return i;
    }
    return -1;
  };
  const start = find([13, 10, 13, 10], 0) + 4;
  const end = find(new TextEncoder().encode(`\r\n--${boundary}`), start);
  return data.slice(start, end < 0 ? data.length : end);
}

export const FRAME_ACCEPT = 'multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1';

// one frame, as modality values (mono) or RGB bytes
export async function loadFrame(instanceUrl, frame, info) {
  const response = await authFetch(new URL(`${instanceUrl}/frames/${frame}`, WEB), { headers: { Accept: FRAME_ACCEPT } });
  if (!response.ok) throw new HttpError(response.status, `frame: ${response.status}`);
  const raw = firstPart(await response.arrayBuffer(), response.headers.get("content-type") || "");
  const { rows, columns, samples, bitsAllocated, signed, slope, intercept } = info;
  const count = rows * columns;
  if (!count) throw new Error("no image size");
  if (samples === 3) {
    const rgb = new Uint8Array(count * 3);
    for (let i = 0; i < count; i++)
      for (let c = 0; c < 3; c++) rgb[i * 3 + c] = info.planar ? raw[c * count + i] : raw[i * 3 + c];
    return { ...info, rgb, defaultWindow: null };
  }
  let stored;
  if (bitsAllocated === 8) stored = signed ? new Int8Array(raw.buffer, 0, count) : raw;
  else if (bitsAllocated === 16) stored = signed ? new Int16Array(raw.buffer, 0, count) : new Uint16Array(raw.buffer, 0, count);
  else if (bitsAllocated === 32) stored = signed ? new Int32Array(raw.buffer, 0, count) : new Uint32Array(raw.buffer, 0, count);
  else throw new Error(`${bitsAllocated}-bit pixels`);
  const values = new Float32Array(count);
  let min = Infinity, max = -Infinity;
  for (let i = 0; i < count; i++) {
    const v = stored[i] * slope + intercept;
    values[i] = v;
    if (v < min) min = v;
    if (v > max) max = v;
  }
  const defaultWindow = info.window || { center: (min + max) / 2, width: Math.max(max - min, 1) };
  return { ...info, values, min, max, defaultWindow };
}

// draw `image` on `canvas` with a window; the canvas keeps the pixel aspect from Pixel Spacing
export function renderImage(canvas, image, wl, invert, maxSize) {
  const { rows, columns } = image;
  const pixels = new ImageData(columns, rows);
  const out = pixels.data;
  if (image.rgb) {
    for (let i = 0, j = 0; i < rows * columns; i++, j += 4) {
      out[j] = image.rgb[i * 3];
      out[j + 1] = image.rgb[i * 3 + 1];
      out[j + 2] = image.rgb[i * 3 + 2];
      out[j + 3] = 255;
    }
  } else {
    const low = wl.center - wl.width / 2;
    const scale = 255 / Math.max(wl.width, 1e-6);
    const flip = invert !== (image.photometric === "MONOCHROME1");
    const v = image.values;
    for (let i = 0, j = 0; i < v.length; i++, j += 4) {
      let g = (v[i] - low) * scale;
      g = g < 0 ? 0 : g > 255 ? 255 : g;
      if (flip) g = 255 - g;
      out[j] = out[j + 1] = out[j + 2] = g;
      out[j + 3] = 255;
    }
  }
  const source = new OffscreenCanvas(columns, rows);
  source.getContext("2d").putImageData(pixels, 0, 0);
  if (maxSize) {
    // a thumbnail: fixed size, image letterboxed
    const [dy, dx] = image.spacing;
    const aspect = (columns * (dx || 1)) / (rows * (dy || 1));
    canvas.width = canvas.height = maxSize;
    const w = aspect >= 1 ? maxSize : maxSize * aspect;
    const hh = aspect >= 1 ? maxSize / aspect : maxSize;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, maxSize, maxSize);
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(source, (maxSize - w) / 2, (maxSize - hh) / 2, w, hh);
  }
  return source;
}

