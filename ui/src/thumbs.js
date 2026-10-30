// Series thumbnails: the middle image of a series, decoded here without Cornerstone (the
// server transcodes it to Explicit VR Little Endian), so the worklist stays light.

import { h, api, WEB } from "./util.js";
import { imageInfo, loadFrame, renderImage } from "./pixels.js";

export function kv(label, value) {
  return [h("dt", {}, label), h("dd", {}, value)];
}

export function sortSeries(series) {
  return [...series].sort((a, b) => Number(a.MainDicomTags.SeriesNumber ?? 1e9) - Number(b.MainDicomTags.SeriesNumber ?? 1e9));
}

export function seriesThumb(series, study, current) {
  const t = series.MainDicomTags;
  const img = h("div", { class: "img" }, h("span", {}, "…"));
  thumbnail(img, series, study.uid);
  return h(
    "a",
    { class: "thumb", href: `#/view/${study.id}/${series.ID}`, "aria-current": current ? "true" : null, title: t.SeriesDescription || "" },
    img,
    h("div", { class: "cap" }, h("b", {}, `${t.Modality || "?"} ${t.SeriesNumber ?? ""}`), h("span", {}, String(series.Instances.length))),
    h("div", { class: "desc" }, t.SeriesDescription || "—"),
  );
}

// thumbnails: the middle image of the series, decoded once, drawn when the tile is on screen
export const thumbs = new Map(); // series ID -> Promise of a decoded image
export const thumbQueue = [];
let thumbsRunning = 0;
export const thumbJobs = new WeakMap();
export const thumbObserver = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      thumbObserver.unobserve(entry.target);
      thumbQueue.push(thumbJobs.get(entry.target));
    }
    pumpThumbs();
  },
  { rootMargin: "200px" },
);
export function thumbnail(el, series, studyUid) {
  thumbJobs.set(el, async () => {
    if (!thumbs.has(series.ID)) {
      thumbs.set(
        series.ID,
        (async () => {
          const instances = await api(`series/${series.ID}/instances`);
          const middle = instances[Math.floor(instances.length / 2)];
          const base = `studies/${studyUid}/series/${series.MainDicomTags.SeriesInstanceUID}/instances/${middle.MainDicomTags.SOPInstanceUID}`;
          const [meta] = await api(`${base}/metadata`, {}, WEB);
          const info = imageInfo(meta);
          return loadFrame(base, Math.ceil(info.frames / 2) || 1, info);
        })(),
      );
    }
    try {
      const image = await thumbs.get(series.ID);
      const canvas = h("canvas");
      renderImage(canvas, image, image.defaultWindow, false, 192);
      el.replaceChildren(canvas);
    } catch {
      thumbs.delete(series.ID);
      el.replaceChildren(h("span", {}, "no preview"));
    }
  });
  thumbObserver.observe(el);
}
export function pumpThumbs() {
  while (thumbsRunning < 3 && thumbQueue.length) {
    thumbsRunning++;
    thumbQueue.shift()().finally(() => {
      thumbsRunning--;
      pumpThumbs();
    });
  }
}

