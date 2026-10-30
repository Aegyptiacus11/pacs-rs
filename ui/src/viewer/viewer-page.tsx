// The viewer screen: full screen and dark. A toolbar on top (tools, then menus that group the
// rest), the study's series on the left, the viewports, and the active viewport's position
// at the bottom. Every action has a tooltip naming its key.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Angle,
  ArrowLeft,
  ArrowLeftRight,
  ArrowUpDown,
  Check,
  ChevronDown,
  Contrast,
  Crosshair,
  Diameter,
  Download,
  Ellipsis,
  Eraser,
  Eye,
  EyeOff,
  Keyboard,
  LayoutGrid,
  Link2,
  Maximize,
  Move,
  Pause,
  Play,
  RotateCw,
  Ruler,
  Spline,
  Square,
  Circle,
  SunMedium,
  Tags,
  Undo2,
  ZoomIn,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { SeriesThumb } from "@/components/series-thumb";
import { api, formatDate, personName } from "@/lib/api";
import { tagRows } from "@/lib/dicom-tags";
import { href } from "@/lib/route";
import { sortSeries } from "@/lib/studies";
import { cn } from "@/lib/utils";
import { initCornerstone, LAYOUTS, PRESETS, Viewer, type ToolName, type ViewerState } from "@/viewer/controller";

const MEASURE: { tool: ToolName; label: string; key: string; Icon: typeof Ruler; hint: string }[] = [
  { tool: "length", label: "Length", key: "L", Icon: Ruler, hint: "Distance in mm" },
  { tool: "probe", label: "Pixel value", key: "H", Icon: Crosshair, hint: "HU at a point" },
  { tool: "angle", label: "Angle", key: "A", Icon: Angle, hint: "Between two lines" },
  { tool: "cobb", label: "Cobb angle", key: "C", Icon: Spline, hint: "Between two separate lines" },
  { tool: "bidirectional", label: "Bidirectional", key: "B", Icon: Diameter, hint: "Long and short axis" },
  { tool: "ellipse", label: "Elliptical ROI", key: "R", Icon: Circle, hint: "Area, mean, std" },
  { tool: "rectangle", label: "Rectangle ROI", key: "G", Icon: Square, hint: "Area, mean, std" },
];

const SHORTCUTS: [string, string][] = [
  ["Wheel, ↑ ↓", "Previous or next image"],
  ["Page Up / Down", "Ten images back or forward"],
  ["[ ]", "Previous or next series"],
  ["W", "Window/level tool"],
  ["P / Z", "Pan / zoom tool"],
  ["L H A C B R G", "Measurement tools"],
  ["Shift Delete", "Remove all measurements"],
  ["1 – 6", "Window presets"],
  ["I", "Invert"],
  ["E", "Rotate 90°"],
  ["Space", "Play or stop"],
  ["K", "Link scrolling"],
  ["O", "Show or hide image text"],
  ["T", "DICOM tags"],
  ["F", "Full screen"],
  ["Double click", "Reset the view"],
  ["Middle drag, Shift drag", "Pan, whatever tool is chosen"],
  ["Right drag", "Zoom, whatever tool is chosen"],
  ["Esc", "Back to the studies"],
];

function ToolButton({ label, shortcut, active, onClick, children, className }: { label: string; shortcut?: string; active?: boolean; onClick?: () => void; children: ReactNode; className?: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={label}
          aria-pressed={active}
          onClick={onClick}
          className={cn("text-muted-foreground hover:text-foreground", active && "bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection", className)}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom" className="flex items-center gap-2">
        {label}
        {shortcut && <Kbd>{shortcut}</Kbd>}
      </TooltipContent>
    </Tooltip>
  );
}

function MenuButton({ label, children, active, Icon, text }: { label: string; children: ReactNode; active?: boolean; Icon: typeof Ruler; text?: string }) {
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              aria-label={label}
              className={cn("gap-1 px-2 text-muted-foreground hover:text-foreground", active && "bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection")}
            >
              <Icon />
              {text && <span className="hidden xl:inline">{text}</span>}
              <ChevronDown className="size-3 opacity-60" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">{label}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start" className="w-60">
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ViewerPage({ study: studyId, series: seriesId }: { study: string; series: string }) {
  const data = useQuery({
    queryKey: ["viewer", studyId],
    queryFn: async () => {
      const [study, series] = await Promise.all([api(`studies/${studyId}`), api<any[]>(`studies/${studyId}/series`), initCornerstone()]);
      return { study, series: sortSeries(series) };
    },
  });
  const grid = useRef<HTMLDivElement>(null);
  const viewer = useRef<Viewer | null>(null);
  const [state, setState] = useState<ViewerState | null>(null);
  const [tags, setTags] = useState(false);
  const [tagQuery, setTagQuery] = useState("");
  const [help, setHelp] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);

  // the engine lives as long as the study is open; the series in the URL is the first shown
  useEffect(() => {
    if (!data.data || !grid.current) return;
    const v = new Viewer(grid.current, data.data.study, data.data.series, setState);
    viewer.current = v;
    v.start(data.data.series.some((s) => s.ID === seriesId) ? seriesId : data.data.series[0]?.ID);
    return () => {
      v.destroy();
      viewer.current = null;
    };
  }, [data.data]);

  // the URL follows the active viewport's series, without a page change
  useEffect(() => {
    const id = state?.shown[state.active];
    if (id && !location.hash.endsWith(id)) history.replaceState(null, "", href.view(studyId, id));
  }, [state?.shown, state?.active]);

  // dark everywhere while the viewer is open, menus and dialogs included (they render outside)
  useEffect(() => {
    document.documentElement.classList.add("dark");
    return () => document.documentElement.classList.remove("dark");
  }, []);

  useEffect(() => {
    const change = () => setFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", change);
    return () => document.removeEventListener("fullscreenchange", change);
  }, []);
  const toggleFullscreen = () => (document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen?.());

  useEffect(() => {
    const keys = (e: KeyboardEvent) => {
      const v = viewer.current;
      if (!v || e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement;
      if (target.closest("input, textarea, [role=dialog], [role=menu]")) return;
      const tool: Record<string, ToolName> = { w: "windowLevel", p: "pan", z: "zoom", l: "length", h: "probe", a: "angle", c: "cobb", b: "bidirectional", r: "ellipse", g: "rectangle" };
      const step: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1, PageDown: 10, PageUp: -10 };
      if (step[e.key]) {
        e.preventDefault();
        v.step(step[e.key]);
      } else if (e.key === "Home") v.first();
      else if (e.key === "End") v.last();
      else if (e.key === "Delete" && e.shiftKey) v.clearMeasurements();
      else if (tool[e.key]) v.useTool(tool[e.key]);
      else if (e.key >= "1" && e.key <= String(PRESETS.length)) v.setPreset(Number(e.key) - 1);
      else if (e.key === "i") v.toggleInvert();
      else if (e.key === "e") v.rotate();
      else if (e.key === " ") {
        e.preventDefault();
        v.cine();
      } else if (e.key === "k") v.setLinked(!state?.linked);
      else if (e.key === "o") v.setOverlays(!state?.overlays);
      else if (e.key === "t") setTags((x) => !x);
      else if (e.key === "f") toggleFullscreen();
      else if (e.key === "[") v.nextSeries(-1);
      else if (e.key === "]") v.nextSeries(1);
      else if (e.key === "?") setHelp(true);
      else if (e.key === "Escape" && !document.fullscreenElement) location.hash = href.studies(null, studyId);
    };
    addEventListener("keydown", keys);
    return () => removeEventListener("keydown", keys);
  }, [state?.linked, state?.overlays, studyId]);

  const v = viewer.current;
  const s = state;
  const study = data.data?.study;
  const allSeries = data.data?.series ?? [];
  const active = s?.shown[s.active];
  const measuring = MEASURE.find((m) => m.tool === s?.tool);
  const rows = useMemo(() => (s?.meta ? tagRows(s.meta, tagQuery.toLowerCase()) : []), [s?.meta, tagQuery]);

  return (
    <div className="dark fixed inset-0 z-50 flex flex-col bg-background text-foreground">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b bg-card px-2">
        <Button variant="ghost" size="sm" asChild className="text-muted-foreground hover:text-foreground">
          <a href={href.studies(null, studyId)}>
            <ArrowLeft /> Studies
          </a>
        </Button>
        <Separator orientation="vertical" className="mx-1 h-5" />
        <div className="mr-auto min-w-0 px-2">
          {study ? (
            <div className="flex items-baseline gap-2 truncate text-sm">
              <span className="font-medium">{personName(study.PatientMainDicomTags.PatientName)}</span>
              <span className="truncate text-muted-foreground">
                {study.MainDicomTags.StudyDescription} · {formatDate(study.MainDicomTags.StudyDate)}
              </span>
            </div>
          ) : (
            <Skeleton className="h-4 w-56" />
          )}
        </div>

        {s && v && (
          <>
            <div className="flex items-center rounded-md">
              <ToolButton label="Window/level: drag to change brightness and contrast" shortcut="W" active={s.tool === "windowLevel"} onClick={() => v.useTool("windowLevel")}>
                <Contrast />
              </ToolButton>
              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon-sm" aria-label="Window presets" className="-ml-1 w-5 text-muted-foreground hover:text-foreground">
                        <ChevronDown className="size-3" />
                      </Button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent side="bottom">Window presets</TooltipContent>
                </Tooltip>
                <DropdownMenuContent align="start" className="w-56">
                  <DropdownMenuLabel>Window</DropdownMenuLabel>
                  {PRESETS.map((p, i) => (
                    <DropdownMenuItem key={p.name} onSelect={() => v.setPreset(i)}>
                      <Check className={cn(s.preset !== i && "opacity-0")} />
                      {p.name}
                      {p.window && <span className="ml-auto text-xs text-muted-foreground tabular">{p.window.width}/{p.window.center}</span>}
                      <DropdownMenuShortcut className="ml-2">{i + 1}</DropdownMenuShortcut>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <ToolButton label="Pan" shortcut="P" active={s.tool === "pan"} onClick={() => v.useTool("pan")}>
              <Move />
            </ToolButton>
            <ToolButton label="Zoom" shortcut="Z" active={s.tool === "zoom"} onClick={() => v.useTool("zoom")}>
              <ZoomIn />
            </ToolButton>
            <MenuButton label="Measure" Icon={measuring?.Icon ?? Ruler} text={measuring?.label ?? "Measure"} active={!!measuring}>
              <DropdownMenuLabel>Measure</DropdownMenuLabel>
              {MEASURE.map((m) => (
                <DropdownMenuItem key={m.tool} onSelect={() => v.useTool(m.tool)}>
                  <m.Icon />
                  <div className="flex flex-col">
                    <span>{m.label}</span>
                    <span className="text-xs text-muted-foreground">{m.hint}</span>
                  </div>
                  <DropdownMenuShortcut>{m.key}</DropdownMenuShortcut>
                </DropdownMenuItem>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => v.clearMeasurements()}>
                <Eraser /> Remove all measurements <DropdownMenuShortcut>⇧⌫</DropdownMenuShortcut>
              </DropdownMenuItem>
            </MenuButton>

            <Separator orientation="vertical" className="mx-1 h-5" />

            <MenuButton label="Layout" Icon={LayoutGrid} text={LAYOUTS[s.layout].name} active={s.layout > 0}>
              <DropdownMenuLabel>Viewports</DropdownMenuLabel>
              <div className="grid grid-cols-4 gap-1 p-1">
                {LAYOUTS.map((l, i) => (
                  <button
                    key={l.name}
                    aria-label={`Layout ${l.name}`}
                    onClick={() => v.setLayout(i)}
                    className={cn("flex flex-col items-center gap-1 rounded-md p-2 text-[11px] text-muted-foreground hover:bg-accent", s.layout === i && "bg-selection-soft text-selection")}
                  >
                    <span className="grid size-8 gap-0.5" style={{ gridTemplateColumns: `repeat(${l.cols}, 1fr)`, gridTemplateRows: `repeat(${l.rows}, 1fr)` }}>
                      {Array.from({ length: l.cols * l.rows }, (_, j) => (
                        <span key={j} className="rounded-[2px] border border-current" />
                      ))}
                    </span>
                    {l.name}
                  </button>
                ))}
              </div>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={(e) => (e.preventDefault(), v.setLinked(!s.linked))} disabled={s.shown.length < 2}>
                <Link2 />
                <div className="flex flex-col">
                  <span>Link scrolling</span>
                  <span className="text-xs text-muted-foreground">Same slice position in every viewport</span>
                </div>
                <Check className={cn("ml-auto", !s.linked && "opacity-0")} />
              </DropdownMenuItem>
            </MenuButton>
            <MenuButton label="Orientation" Icon={RotateCw}>
              <DropdownMenuItem onSelect={() => v.rotate()}>
                <RotateCw /> Rotate 90° <DropdownMenuShortcut>E</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.flip("horizontal")}>
                <ArrowLeftRight /> Flip left to right
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.flip("vertical")}>
                <ArrowUpDown /> Flip top to bottom
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.toggleInvert()}>
                <SunMedium /> Invert <Check className={cn("ml-auto", !s.invert && "opacity-0")} />
                <DropdownMenuShortcut>I</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => v.reset()}>
                <Undo2 /> Reset view <DropdownMenuShortcut>dbl-click</DropdownMenuShortcut>
              </DropdownMenuItem>
            </MenuButton>
            <ToolButton label={s.playing ? "Stop" : "Play through the series"} shortcut="Space" active={s.playing} onClick={() => v.cine()}>
              {s.playing ? <Pause /> : <Play />}
            </ToolButton>
            {s.shown.length > 1 && (
              <ToolButton label="Link scrolling" shortcut="K" active={s.linked} onClick={() => v.setLinked(!s.linked)}>
                <Link2 />
              </ToolButton>
            )}

            <Separator orientation="vertical" className="mx-1 h-5" />

            <ToolButton label="DICOM tags" shortcut="T" active={tags} onClick={() => setTags(!tags)}>
              <Tags />
            </ToolButton>
            <DropdownMenu>
              <Tooltip>
                <TooltipTrigger asChild>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon-sm" aria-label="More" className="text-muted-foreground hover:text-foreground">
                      <Ellipsis />
                    </Button>
                  </DropdownMenuTrigger>
                </TooltipTrigger>
                <TooltipContent side="bottom">More</TooltipContent>
              </Tooltip>
              <DropdownMenuContent align="end" className="w-60">
                <DropdownMenuItem asChild disabled={!s.instanceFile}>
                  <a href={s.instanceFile ?? undefined} download>
                    <Download /> Download this image (.dcm)
                  </a>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => v.setOverlays(!s.overlays)}>
                  {s.overlays ? <EyeOff /> : <Eye />} {s.overlays ? "Hide" : "Show"} image text <DropdownMenuShortcut>O</DropdownMenuShortcut>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={toggleFullscreen}>
                  <Maximize /> {fullscreen ? "Leave full screen" : "Full screen"} <DropdownMenuShortcut>F</DropdownMenuShortcut>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setHelp(true)}>
                  <Keyboard /> Keyboard shortcuts <DropdownMenuShortcut>?</DropdownMenuShortcut>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        )}
      </header>

      <div className="flex min-h-0 flex-1">
        <aside className="hidden w-48 shrink-0 flex-col border-r bg-card md:flex">
          <div className="flex h-10 items-center justify-between px-3 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">Series</span>
            <span className="tabular">{allSeries.length || ""}</span>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            <div className="space-y-2 px-3 pb-3">
              {data.isPending && Array.from({ length: 3 }, (_, i) => <Skeleton key={i} className="aspect-square w-full rounded-md" />)}
              {allSeries.map((x) => {
                const isActive = x.ID === active;
                const shown = s?.shown.includes(x.ID);
                return (
                  <button
                    key={x.ID}
                    onClick={() => v?.showSeries(x.ID)}
                    className={cn("group block w-full rounded-lg p-1.5 text-left transition-colors hover:bg-accent", isActive && "bg-selection-soft hover:bg-selection-soft")}
                  >
                    <SeriesThumb series={x} studyUid={study.MainDicomTags.StudyInstanceUID} className={cn("ring-2 ring-transparent", isActive && "ring-selection", !isActive && shown && "ring-selection/30")} />
                    <div className="mt-1.5 truncate px-0.5 text-xs font-medium">{x.MainDicomTags.SeriesDescription || "No description"}</div>
                    <div className="flex justify-between px-0.5 text-[11px] text-muted-foreground">
                      <span>
                        {x.MainDicomTags.Modality} {x.MainDicomTags.SeriesNumber ? `#${x.MainDicomTags.SeriesNumber}` : ""}
                      </span>
                      <span className="tabular">{x.Instances.length}</span>
                    </div>
                  </button>
                );
              })}
            </div>
          </ScrollArea>
        </aside>

        <main className="flex min-w-0 flex-1 flex-col">
          <div ref={grid} className="viewer-grid min-h-0 flex-1" data-tool={s?.tool} />
          <div className="flex h-9 shrink-0 items-center gap-3 border-t bg-card px-4">
            <input
              type="range"
              aria-label="Image"
              min={0}
              max={Math.max((s?.count ?? 1) - 1, 0)}
              value={s?.index ?? 0}
              onChange={(e) => v?.go(Number(e.target.value))}
              className="h-1 flex-1 cursor-pointer accent-[var(--selection)]"
            />
            <span className="tabular w-16 text-right text-xs text-muted-foreground">{s?.count ? `${s.index + 1} / ${s.count}` : ""}</span>
            <Button variant="ghost" size="icon-xs" aria-label="Keyboard shortcuts" className="text-muted-foreground" onClick={() => setHelp(true)}>
              <Keyboard />
            </Button>
          </div>
        </main>
      </div>

      <Sheet open={tags} onOpenChange={setTags} modal={false}>
        <SheetContent className="w-[480px] gap-0 sm:max-w-[480px]" onInteractOutside={(e) => e.preventDefault()}>
          <SheetHeader className="border-b">
            <SheetTitle>DICOM tags</SheetTitle>
            <SheetDescription>The image shown in the active viewport</SheetDescription>
            <Input value={tagQuery} onChange={(e) => setTagQuery(e.target.value)} placeholder="Filter by tag, name or value" className="mt-2 h-8" />
          </SheetHeader>
          <ScrollArea className="min-h-0 flex-1">
            <table className="w-full text-xs">
              <tbody>
                {rows.map((r, i) => (
                  <tr key={i} className="border-b align-top">
                    <td className="py-1.5 pr-2 pl-4 font-mono whitespace-nowrap text-muted-foreground" style={{ paddingLeft: 16 + r.depth * 14 }}>
                      {r.tag}
                    </td>
                    <td className="w-1/3 py-1.5 pr-2 text-muted-foreground">{r.name}</td>
                    <td className="py-1.5 pr-4 font-mono break-all">{r.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </ScrollArea>
        </SheetContent>
      </Sheet>

      <Dialog open={help} onOpenChange={setHelp}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Keyboard and mouse</DialogTitle>
            <DialogDescription>The same keys work in every viewport; they act on the active one.</DialogDescription>
          </DialogHeader>
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
            {SHORTCUTS.map(([k, what]) => (
              <div key={k} className="contents">
                <dt>
                  <Kbd>{k}</Kbd>
                </dt>
                <dd className="text-muted-foreground">{what}</dd>
              </div>
            ))}
          </dl>
        </DialogContent>
      </Dialog>
    </div>
  );
}
