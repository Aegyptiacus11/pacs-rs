// DICOM Segmentation (SEG) and RT Structure Set (RTSTRUCT) objects read from their files, for
// the viewer to lay over the images they were drawn on: a SEG's segments as masks, one per
// referenced image, and an RTSTRUCT's regions as contours in patient coordinates.
//
// SEG (PS3.3 A.51, C.8.20): segments in the Segment Sequence, each frame a mask of one segment
// on one source image, named by the frame's Derivation Image Sequence (or, without one, by its
// Plane Position). BINARY frames are one bit a pixel, packed with no gap between frames
// (PS3.5 8.1.1); FRACTIONAL and LABELMAP frames are a byte a pixel. Only files stored
// uncompressed are read: a compressed SEG is refused with that reason.
//
// RTSTRUCT (PS3.3 A.19, C.8.8.5-6): regions of interest in the Structure Set ROI Sequence,
// their contours in the ROI Contour Sequence, each a list of points in mm (LPS) on one image.
//
// No path alias here: the browser tests import this module as it is.

import dicomParser from "dicom-parser";

/** Transfer syntaxes whose pixel data is read as it lies. */
const UNCOMPRESSED: Record<string, boolean> = {
  "1.2.840.10008.1.2": true, // Implicit VR Little Endian
  "1.2.840.10008.1.2.1": true, // Explicit VR Little Endian
};

export type Rgb = [number, number, number];

export type Segment = { number: number; label: string; color: Rgb };

/** One frame: which segment, on which image (and frame of it), where its first pixel is, how
 * its rows and columns run (row direction, then column direction) and how far apart its
 * pixels are (between rows, between columns, mm). A SEG has a geometry of its own, which
 * need not be its source image's: rows may run the other way. */
export type SegFrame = { segment: number; sop: string | null; frame: number; position: number[] | null; orientation: number[] | null; spacing: number[] | null };

export type Seg = {
  kind: "SEG";
  sop: string;
  /** the series its images are in */
  referencedSeries: string[];
  segments: Segment[];
  frames: SegFrame[];
  rows: number;
  columns: number;
  /** the frame's mask, a byte a pixel: 1 where the segment is */
  mask(frame: number): Uint8Array;
};

export type Contour = { sop: string | null; points: number[][]; closed: boolean };
export type Roi = { number: number; name: string; color: Rgb; contours: Contour[] };

export type RtStruct = {
  kind: "RTSTRUCT";
  sop: string;
  label: string;
  referencedSeries: string[];
  rois: Roi[];
};

export class OverlayError extends Error {}

function parse(buffer: ArrayBuffer) {
  try {
    return dicomParser.parseDicom(new Uint8Array(buffer));
  } catch (e: any) {
    // dicom-parser throws a string, or an object with one
    throw new OverlayError(`not a DICOM file: ${typeof e === "string" ? e : (e?.exception ?? e?.message ?? e)}`);
  }
}

const text = (ds: any, tag: string) => (ds.string(tag) ?? "").trim();
const items = (ds: any, tag: string): any[] => (ds.elements[tag]?.items ?? []).map((i: any) => i.dataSet);
const numbers = (ds: any, tag: string): number[] => {
  const s = ds.string(tag);
  return s ? s.split("\\").map(Number) : [];
};

/** CIELab as DICOM scales it (PS3.3 C.10.7.1.1: L 0-100, a and b -128-127 over 0-65535)
 * to sRGB, under D65. */
export function labToRgb([l16, a16, b16]: number[]): Rgb {
  const L = (l16 / 65535) * 100, a = (a16 / 65535) * 255 - 128, b = (b16 / 65535) * 255 - 128;
  const fy = (L + 16) / 116, fx = fy + a / 500, fz = fy - b / 200;
  const f = (t: number) => (t ** 3 > 0.008856 ? t ** 3 : (t - 16 / 116) / 7.787);
  const [X, Y, Z] = [0.95047 * f(fx), 1 * f(fy), 1.08883 * f(fz)];
  const lin = [3.2406 * X - 1.5372 * Y - 0.4986 * Z, -0.9689 * X + 1.8758 * Y + 0.0415 * Z, 0.0557 * X - 0.204 * Y + 1.057 * Z];
  return lin.map((c) => {
    const s = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(1, Math.max(0, s)) * 255);
  }) as Rgb;
}

// what a segment or region with no color of its own gets, in turn
const PALETTE: Rgb[] = [
  [230, 25, 75],
  [60, 180, 75],
  [255, 225, 25],
  [0, 130, 200],
  [245, 130, 48],
  [145, 30, 180],
  [70, 240, 240],
  [240, 50, 230],
];

export function readOverlay(buffer: ArrayBuffer): Seg | RtStruct {
  const ds = parse(buffer);
  const modality = text(ds, "x00080060");
  if (modality === "SEG") return readSeg(ds);
  if (modality === "RTSTRUCT") return readRtStruct(ds);
  throw new OverlayError(`a ${modality || "file with no modality"}, neither a SEG nor an RTSTRUCT`);
}

function readSeg(ds: any): Seg {
  const syntax = text(ds, "x00020010");
  if (!UNCOMPRESSED[syntax]) throw new OverlayError(`this segmentation is stored compressed (${syntax}), which the viewer cannot read yet`);
  const type = text(ds, "x00620001");
  const rows = ds.uint16("x00280010"), columns = ds.uint16("x00280011");
  const bits = ds.uint16("x00280100");
  const count = Number(text(ds, "x00280008") || 1);
  const pixels = ds.elements.x7fe00010;
  if (!rows || !columns || !pixels) throw new OverlayError("this segmentation has no pixel data");
  if (type === "BINARY" ? bits !== 1 : bits !== 8) throw new OverlayError(`a ${type || "segmentation"} of ${bits} bits a pixel, which the viewer cannot read`);
  const size = rows * columns;
  const need = type === "BINARY" ? Math.ceil((size * count) / 8) : size * count;
  if (pixels.length < need) throw new OverlayError(`its pixel data holds ${pixels.length} bytes, ${count} frames need ${need}`);

  const segments: Segment[] = items(ds, "x00620002").map((s, i) => {
    const lab = numbers(s, "x0062000d");
    return {
      number: s.uint16("x00620004"),
      label: text(s, "x00620005") || `Segment ${i + 1}`,
      color: lab.length === 3 ? labToRgb(lab) : PALETTE[i % PALETTE.length],
    };
  });
  const shared = items(ds, "x52009229")[0];
  const perFrame = items(ds, "x52009230");
  if (perFrame.length !== count) throw new OverlayError(`${count} frames, but ${perFrame.length} described`);
  const segmentOf = (g: any) => items(g, "x0062000a")[0]?.uint16("x0062000b");
  // in the frame's own functional groups, else in the shared ones
  const inGroups = (g: any, sequence: string, tag: string, n: number) => {
    for (const group of [g, shared]) {
      const v = group ? numbers(items(group, sequence)[0] ?? { string: () => undefined }, tag) : [];
      if (v.length === n && v.every(Number.isFinite)) return v;
    }
    return null;
  };
  const frames: SegFrame[] = perFrame.map((g) => {
    const source = items(items(g, "x00089124")[0] ?? { elements: {} }, "x00082112")[0];
    const position = numbers(items(g, "x00209113")[0] ?? { string: () => undefined }, "x00200032");
    return {
      segment: segmentOf(g) ?? (shared ? segmentOf(shared) : undefined) ?? 1,
      sop: source ? text(source, "x00081155") || null : null,
      frame: source ? Number(text(source, "x00081160") || 1) : 1,
      position: position.length === 3 ? position : null,
      orientation: inGroups(g, "x00209116", "x00200037", 6),
      spacing: inGroups(g, "x00289110", "x00280030", 2),
    };
  });
  const bytes = new Uint8Array(ds.byteArray.buffer, ds.byteArray.byteOffset + pixels.dataOffset, pixels.length);
  // FRACTIONAL: a probability or occupancy up to Maximum Fractional Value (0062,000E);
  // half of it is in
  const max = ds.uint16("x0062000e") || 255;
  return {
    kind: "SEG",
    sop: text(ds, "x00080018"),
    referencedSeries: items(ds, "x00081115").map((s) => text(s, "x0020000e")).filter(Boolean),
    segments,
    frames,
    rows,
    columns,
    mask(f: number) {
      const out = new Uint8Array(size);
      if (type === "BINARY") {
        // bit n of the pixel data is pixel n of the frames end to end, least significant first
        let bit = f * size;
        for (let p = 0; p < size; p++, bit++) out[p] = (bytes[bit >> 3] >> (bit & 7)) & 1;
      } else {
        const from = f * size;
        for (let p = 0; p < size; p++) out[p] = type === "FRACTIONAL" ? +(bytes[from + p] * 2 >= max && bytes[from + p] > 0) : +(bytes[from + p] > 0);
      }
      return out;
    },
  };
}

function readRtStruct(ds: any): RtStruct {
  const names = new Map<number, string>(items(ds, "x30060020").map((r) => [Number(text(r, "x30060022")), text(r, "x30060026")]));
  const rois: Roi[] = items(ds, "x30060039").map((rc, i) => {
    const number = Number(text(rc, "x30060084"));
    const color = numbers(rc, "x3006002a");
    return {
      number,
      name: names.get(number) || `Region ${number}`,
      color: color.length === 3 ? (color as Rgb) : PALETTE[i % PALETTE.length],
      contours: items(rc, "x30060040")
        .map((c) => {
          const flat = numbers(c, "x30060050");
          const points: number[][] = [];
          for (let p = 0; p + 2 < flat.length; p += 3) points.push([flat[p], flat[p + 1], flat[p + 2]]);
          const type = text(c, "x30060042");
          return { sop: text(items(c, "x30060016")[0] ?? { string: () => undefined }, "x00081155") || null, points, closed: type === "CLOSED_PLANAR" };
        })
        .filter((c) => c.points.length > 0),
    };
  });
  const series = items(ds, "x30060010").flatMap((f) => items(f, "x30060012").flatMap((st) => items(st, "x30060014").map((se) => text(se, "x0020000e"))));
  return { kind: "RTSTRUCT", sop: text(ds, "x00080018"), label: text(ds, "x30060002"), referencedSeries: series.filter(Boolean), rois };
}

// ---- placing them on images ----

/** Where an image lies: its first pixel's position, its row and column directions, the
 * spacing between rows and between columns (mm), and its normal. */
export type Plane = { origin: number[]; row: number[]; column: number[]; spacing: [number, number]; normal: number[]; rows: number; columns: number };

const dot = (a: number[], b: number[]) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const sub = (a: number[], b: number[]) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];

/** One segment's mask on one image, in that image's pixels: 1 where the segment is. */
export type Layer = { segment: number; mask: Uint8Array };

/** The masks of a SEG laid on the images of its series, as the viewer draws them: for each
 * referenced image, a mask a segment on it, in that image's pixels, in rising segment number
 * (overlapping segments each keep their own, so hiding one shows what lies under it). Images
 * are found by SOP Instance UID and frame, or, for frames that name none, by the plane they
 * lie in. A frame whose geometry is known is sampled at each image pixel it covers (nearest
 * SEG pixel), so a SEG coarser than its images, or turned against them, has no holes; one
 * without is taken as lying pixel for pixel on its image. Frames on no image of the series
 * are counted, not drawn. */
export function segOnImages(seg: Seg, images: { sop: string; frame: number; plane: Plane | null }[]) {
  const bySop = new Map(images.map((im, i) => [`${im.sop}/${im.frame}`, i]));
  const out: (Layer[] | null)[] = images.map(() => null);
  let missed = 0;
  // lowest segment first: each image's layers in rising number, a higher one drawn on top
  const order = seg.frames.map((f, i) => [f, i] as const).sort((a, b) => a[0].segment - b[0].segment);
  for (const [f, i] of order) {
    let at = f.sop ? bySop.get(`${f.sop}/${f.frame}`) : undefined;
    if (at === undefined && f.position) at = onPlane(images, f.position);
    if (at === undefined) {
      missed++;
      continue;
    }
    const plane = images[at].plane;
    const rows = plane?.rows ?? seg.rows, columns = plane?.columns ?? seg.columns;
    const mask = seg.mask(i);
    const layers = (out[at] ??= []);
    let layer = layers.find((l) => l.segment === f.segment);
    if (!layer) layers.push((layer = { segment: f.segment, mask: new Uint8Array(rows * columns) }));
    const target = layer.mask;
    const map = plane && f.position && f.orientation && f.spacing ? carry(f, plane) : null;
    if (!map) {
      if (rows !== seg.rows || columns !== seg.columns) {
        missed++;
        continue;
      }
      for (let p = 0; p < mask.length; p++) if (mask[p]) target[p] = 1;
      continue;
    }
    // SEG pixel (c, r) -> image pixel (x, y) = base + c * dc + r * dr; each image pixel
    // within the frame's outline takes the SEG pixel it falls in, by the inverse
    const [bx, by, cx, cy, rx, ry] = map;
    const det = cx * ry - rx * cy;
    if (Math.abs(det) < 1e-9) {
      // a frame seen edge-on, across the image: it covers no area of it
      missed++;
      continue;
    }
    const corners = [-0.5, seg.columns - 0.5].flatMap((c) => [-0.5, seg.rows - 0.5].map((r) => [bx + c * cx + r * rx, by + c * cy + r * ry]));
    const [x0, x1] = [Math.max(0, Math.floor(Math.min(...corners.map((k) => k[0])))), Math.min(columns - 1, Math.ceil(Math.max(...corners.map((k) => k[0]))))];
    const [y0, y1] = [Math.max(0, Math.floor(Math.min(...corners.map((k) => k[1])))), Math.min(rows - 1, Math.ceil(Math.max(...corners.map((k) => k[1]))))];
    for (let y = y0; y <= y1; y++)
      for (let x = x0; x <= x1; x++) {
        const [u, v] = [x - bx, y - by];
        const c = Math.round((ry * u - rx * v) / det), r = Math.round((cx * v - cy * u) / det);
        if (c >= 0 && r >= 0 && c < seg.columns && r < seg.rows && mask[r * seg.columns + c]) target[y * columns + x] = 1;
      }
  }
  return { layers: out, missed };
}

/** One image's layers as one map: each pixel the highest segment number there, 0 where none
 * is; what a single labelmap (MPR) can hold. */
export function topSegments(layers: Layer[], size: number) {
  const map = new Uint16Array(size);
  for (const { segment, mask } of layers) for (let p = 0; p < size; p++) if (mask[p]) map[p] = segment;
  return map;
}

/** How a SEG frame's pixel indices become an image's: [x0, y0, dx/dc, dy/dc, dx/dr, dy/dr]. */
function carry(f: SegFrame, plane: Plane) {
  const o = f.orientation!, sp = f.spacing!, pos = f.position!;
  const world = (c: number, r: number) => [0, 1, 2].map((k) => pos[k] + c * sp[1] * o[k] + r * sp[0] * o[k + 3]);
  const toImage = (w: number[]) => {
    const d = sub(w, plane.origin);
    return [dot(d, plane.row) / plane.spacing[1], dot(d, plane.column) / plane.spacing[0]];
  };
  const [b, c, r] = [toImage(world(0, 0)), toImage(world(1, 0)), toImage(world(0, 1))];
  return [b[0], b[1], c[0] - b[0], c[1] - b[1], r[0] - b[0], r[1] - b[1]];
}

/** The image whose plane a position lies in (a tenth of a millimetre either way). */
function onPlane(images: { plane: Plane | null }[], position: number[]) {
  let best: number | undefined, distance = Infinity;
  images.forEach((im, i) => {
    if (!im.plane) return;
    const d = Math.abs(dot(sub(position, im.plane.origin), im.plane.normal));
    if (d < distance) [best, distance] = [i, d];
  });
  return distance < 0.1 ? best : undefined;
}

/** What of a contour a plane shows (a point on it, its unit normal, and how thick a slice it
 * stands for, mm): the whole contour when it lies within the slice, else where its edges cross
 * the plane, as line segments (a sagittal or coronal view of axial contours). */
export function contourCut(contour: Contour, point: number[], normal: number[], thickness: number): { polygon: number[][] | null; segments: number[][][] } {
  const side = contour.points.map((p) => dot(sub(p, point), normal));
  if (side.every((d) => Math.abs(d) <= thickness / 2)) return { polygon: contour.points, segments: [] };
  const crossings: number[][] = [];
  const n = contour.points.length;
  for (let i = 0; i < (contour.closed ? n : n - 1); i++) {
    const j = (i + 1) % n;
    const [a, b] = [side[i], side[j]];
    if ((a < 0 && b >= 0) || (a >= 0 && b < 0)) {
      const t = a / (a - b);
      const [p, q] = [contour.points[i], contour.points[j]];
      crossings.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]), p[2] + t * (q[2] - p[2])]);
    }
  }
  // a closed contour's crossings pair up along the line where they lie
  if (crossings.length >= 2) {
    const dir = sub(crossings[crossings.length - 1], crossings[0]);
    crossings.sort((x, y) => dot(x, dir) - dot(y, dir));
  }
  const segments: number[][][] = [];
  for (let i = 0; i + 1 < crossings.length; i += 2) segments.push([crossings[i], crossings[i + 1]]);
  return { polygon: null, segments };
}
