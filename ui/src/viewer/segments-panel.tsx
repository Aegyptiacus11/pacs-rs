// The segmentations laid over the images: each SEG or RTSTRUCT with its segments or regions,
// each shown or hidden, the whole one taken away, and how opaque the fill is.

import { Eye, EyeOff, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { OverlayState, Viewer } from "@/viewer/controller";

export function SegmentsPanel({ viewer, list, fill, onClose }: { viewer: Viewer; list: OverlayState[]; fill: number; onClose: () => void }) {
  return (
    <section aria-label="Segmentations" className="absolute top-2 right-2 z-20 flex max-h-[calc(100%-1rem)] w-72 flex-col rounded-lg border bg-popover/95 text-sm shadow-lg backdrop-blur">
      <div className="flex items-center justify-between border-b py-1.5 pr-1.5 pl-3">
        <h2 className="font-medium">Segmentations</h2>
        <Button variant="ghost" size="icon-sm" aria-label="Close the segmentations" onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 overflow-y-auto">
        {list.map((o) => (
          <div key={o.seriesId} className="border-b px-1.5 py-1.5 last:border-0">
            <div className="flex items-center gap-1">
              <Eyes label={o.label} visible={o.visible} onToggle={() => viewer.setSegmentationVisible(o.seriesId, !o.visible)} />
              <span className="min-w-0 flex-1 truncate font-medium" title={o.label}>
                {o.label}
              </span>
              <span className="font-mono text-[10px] text-muted-foreground">{o.kind}</span>
              <Button variant="ghost" size="icon-xs" aria-label={`Take ${o.label} away`} onClick={() => viewer.removeSegmentation(o.seriesId)}>
                <X />
              </Button>
            </div>
            {!o.targetShown && <p className="px-2 pb-1 text-xs text-muted-foreground">Its series is not in any viewport.</p>}
            {o.missed > 0 && <p className="px-2 pb-1 text-xs text-muted-foreground">{o.missed} of its frames lie on images this archive does not have.</p>}
            <ul aria-label={`${o.kind === "SEG" ? "Segments" : "Regions"} of ${o.label}`} className={cn("pl-5", !o.visible && "opacity-50")}>
              {o.items.map((it) => (
                <li key={it.index} className="flex items-center gap-1.5">
                  <Eyes label={it.label} visible={it.visible} disabled={!o.visible} onToggle={() => viewer.setSegmentationVisible(o.seriesId, !it.visible, it.index)} />
                  <span className="size-3 shrink-0 rounded-sm ring-1 ring-black/20" style={{ background: `rgb(${it.color.join(",")})` }} />
                  <span className="truncate">{it.label}</span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      <label className="flex items-center gap-3 border-t px-3 py-2 text-xs text-muted-foreground">
        Fill
        <input type="range" min={0} max={1} step={0.05} value={fill} onChange={(e) => viewer.setFill(Number(e.target.value))} className="flex-1 accent-[var(--selection)]" aria-label="Fill opacity" />
        <span className="w-8 text-right tabular">{Math.round(fill * 100)}%</span>
      </label>
    </section>
  );
}

function Eyes({ label, visible, disabled, onToggle }: { label: string; visible: boolean; disabled?: boolean; onToggle: () => void }) {
  return (
    <Button variant="ghost" size="icon-xs" aria-label={`${visible ? "Hide" : "Show"} ${label}`} aria-pressed={visible} disabled={disabled} onClick={onToggle} className="text-muted-foreground">
      {visible ? <Eye /> : <EyeOff />}
    </Button>
  );
}
