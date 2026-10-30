// A series' middle image, decoded here without Cornerstone (the server transcodes it to
// Explicit VR Little Endian), once per series, when the tile scrolls into view, three at a time.

import { useEffect, useRef, useState } from "react";
import { api, WEB } from "@/lib/api";
import { imageInfo, loadFrame, renderImage } from "@/lib/pixels";
import { cn } from "@/lib/utils";

const images = new Map<string, Promise<any>>();
const queue: (() => Promise<void>)[] = [];
let running = 0;
function pump() {
  while (running < 3 && queue.length) {
    running++;
    queue.shift()!().finally(() => {
      running--;
      pump();
    });
  }
}

function middleImage(seriesId: string, seriesUid: string, studyUid: string) {
  if (!images.has(seriesId)) {
    images.set(
      seriesId,
      (async () => {
        const instances = await api<any[]>(`series/${seriesId}/instances`);
        const middle = instances[Math.floor(instances.length / 2)];
        const base = `studies/${studyUid}/series/${seriesUid}/instances/${middle.MainDicomTags.SOPInstanceUID}`;
        const [meta] = await api<any[]>(`${base}/metadata`, {}, WEB);
        const info = imageInfo(meta);
        return loadFrame(base, Math.ceil(info.frames / 2) || 1, info);
      })().catch((e) => {
        images.delete(seriesId);
        throw e;
      }),
    );
  }
  return images.get(seriesId)!;
}

export function SeriesThumb({ series, studyUid, className }: { series: any; studyUid: string; className?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<"waiting" | "ready" | "none">("waiting");

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let alive = true;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return;
        observer.disconnect();
        queue.push(async () => {
          try {
            const image = await middleImage(series.ID, series.MainDicomTags.SeriesInstanceUID, studyUid);
            if (!alive || !canvas.current) return;
            renderImage(canvas.current, image, image.defaultWindow, false, 256);
            setState("ready");
          } catch {
            if (alive) setState("none");
          }
        });
        pump();
      },
      { rootMargin: "200px" },
    );
    observer.observe(el);
    return () => {
      alive = false;
      observer.disconnect();
    };
  }, [series.ID, series.MainDicomTags.SeriesInstanceUID, studyUid]);

  return (
    <div ref={box} className={cn("relative aspect-square overflow-hidden rounded-md bg-black", className)}>
      <canvas ref={canvas} className={cn("size-full transition-opacity duration-300", state === "ready" ? "opacity-100" : "opacity-0")} />
      {state === "waiting" && <div className="absolute inset-0 animate-pulse bg-neutral-900" />}
      {state === "none" && <div className="absolute inset-0 grid place-items-center text-[11px] text-neutral-500">No preview</div>}
    </div>
  );
}
