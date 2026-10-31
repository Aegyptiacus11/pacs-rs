// The viewer screen, laid out like a reading workstation: one toolbar of labelled tools (a
// caret where a tool has variants), the patient's studies and series on the left, the
// viewports filling the rest. Every action has a tooltip naming its key.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Angle,
  ArrowLeft,
  ArrowLeftRight,
  ArrowUpDown,
  Axis3d,
  Box,
  Check,
  ChevronDown,
  Circle,
  Contrast,
  Crosshair,
  Diameter,
  Download,
  Ellipsis,
  Eraser,
  ImageDown,
  Info,
  Keyboard,
  LayoutGrid,
  SquarePlus,
  Link2,
  Maximize,
  Minimize,
  Move,
  PanelLeft,
  Pause,
  Pipette,
  Play,
  RotateCcw,
  RotateCw,
  Ruler,
  ScanSearch,
  Spline,
  Square,
  SunMoon,
  Tags,
  ZoomIn,
} from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { toast } from "sonner";
import { Logo } from "@/components/app-header";
import { api } from "@/lib/api";
import { tagRows } from "@/lib/dicom-tags";
import { href } from "@/lib/route";
import { applyTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { initCornerstone, LAYOUTS, PRESETS, renderingMode, setRenderingMode, Viewer, type ToolName, type ViewerState } from "@/viewer/controller";
import { StudyBrowser, type Registry } from "@/viewer/study-browser";
import { autoApply, fetchUserProtocols, plans, studyContext, type Plan, type Protocol, type StudyContext } from "@/viewer/hanging";
import { ProtocolMenu } from "@/viewer/protocol-menu";

// the measurements, each its own toolbar button; keys after their names
const MEASURE: { tool: ToolName; label: string; short: string; key: string; Icon: typeof Ruler; hint: string }[] = [
  { tool: "length", label: "Length", short: "Length", key: "L", Icon: Ruler, hint: "distance in mm" },
  { tool: "angle", label: "Angle", short: "Angle", key: "A", Icon: Angle, hint: "between two lines" },
  { tool: "cobb", label: "Cobb angle", short: "Cobb", key: "C", Icon: Spline, hint: "between two separate lines" },
  { tool: "bidirectional", label: "Bidirectional", short: "Bidir.", key: "B", Icon: Diameter, hint: "long and short axis" },
  { tool: "ellipse", label: "Ellipse ROI", short: "Ellipse", key: "E", Icon: Circle, hint: "area, mean, std" },
  { tool: "rectangle", label: "Rectangle ROI", short: "Rectangle", key: "⇧ E", Icon: Square, hint: "area, mean, std" },
  { tool: "probe", label: "Pixel value", short: "Value", key: "D", Icon: Pipette, hint: "the HU (density) at a point" },
];

const SHORTCUTS: [string, string][] = [
  ["Wheel, ↑ ↓", "Previous or next image"],
  ["Page Up / Down", "Ten images back or forward"],
  ["[ ]", "Previous or next series"],
  ["W", "Window/level"],
  ["P", "Pan"],
  ["Z", "Zoom"],
  ["M", "Magnifier: hold over a spot, wheel for more or less"],
  ["L A C B", "Length, angle, Cobb angle, bidirectional"],
  ["E, ⇧ E", "Ellipse ROI, rectangle ROI"],
  ["D", "Pixel value (density, HU)"],
  ["⇧ Delete", "Remove all measurements"],
  ["R, ⇧ R", "Rotate right, left"],
  ["H, V", "Flip horizontal, vertical"],
  ["I", "Invert"],
  ["0", "Reset the view"],
  ["1 – 6", "Window presets"],
  ["N", "New view"],
  ["X", "3 planes (axial, sagittal, coronal) on or off"],
  ["S", "Sync: scroll the viewports together"],
  ["Space", "Play or stop"],
  ["O", "Show or hide the text on the images"],
  ["T", "DICOM tags"],
  ["F", "Full screen"],
  ["Double click", "Reset the view"],
  ["Drag a series", "Show it in that viewport"],
  ["Middle drag, Shift drag", "Pan, whatever tool is chosen"],
  ["Right drag", "Zoom, whatever tool is chosen"],
  ["In a 3D view", "Drag to turn it, wheel or right drag to zoom"],
  [", .", "Previous or next hanging protocol"],
  ["Esc", "Back to the studies"],
];

type Tb = { label: string; tip?: string; Icon: typeof Ruler; shortcut?: string; active?: boolean; disabled?: boolean; onClick?: () => void };

const toolClass = (active?: boolean) =>
  cn(
    "flex h-12 min-w-11 flex-col items-center justify-center gap-0.5 rounded-md px-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-35",
    active && "bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection",
  );

function Tip({ label, shortcut, children }: { label: string; shortcut?: string; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="bottom" className="flex items-center gap-2">
        {label}
        {shortcut && <Kbd>{shortcut}</Kbd>}
      </TooltipContent>
    </Tooltip>
  );
}

// a tool of the toolbar: the icon over its name
function ToolBtn({ label, tip, Icon, shortcut, active, disabled, onClick, className }: Tb & { className?: string }) {
  return (
    <Tip label={tip ?? label} shortcut={shortcut}>
      {/* a span keeps the tooltip on a disabled button, so it can say why */}
      <span className="inline-flex">
      <button type="button" aria-label={label} aria-pressed={active} disabled={disabled} onClick={onClick} className={cn(toolClass(active), className)}>
        <Icon className="size-5" strokeWidth={1.6} />
        <span className="max-w-16 truncate text-[10.5px] leading-none">{label}</span>
      </button>
      </span>
    </Tip>
  );
}

// a tool with variants: the tool itself, and a caret for the variants; without a tool of its
// own (Layout), the whole button opens them
function ToolMenu({ children, menuLabel, ...tb }: Tb & { children: ReactNode; menuLabel: string }) {
  const menu = <DropdownMenuContent align="start" className="w-60">{children}</DropdownMenuContent>;
  if (!tb.onClick)
    return (
      <DropdownMenu>
        <Tip label={menuLabel} shortcut={tb.shortcut}>
          <DropdownMenuTrigger asChild>
            <button type="button" aria-label={menuLabel} disabled={tb.disabled} className={toolClass(tb.active)}>
              <tb.Icon className="size-5" strokeWidth={1.6} />
              <span className="flex items-center gap-0.5 text-[10.5px] leading-none">
                {tb.label} <ChevronDown className="size-2.5" />
              </span>
            </button>
          </DropdownMenuTrigger>
        </Tip>
        {menu}
      </DropdownMenu>
    );
  return (
    <div className={cn("flex items-stretch rounded-md", tb.active && "bg-selection-soft")}>
      <ToolBtn {...tb} className="rounded-r-none pr-0.5" />
      <DropdownMenu>
        <Tip label={menuLabel}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={menuLabel}
              disabled={tb.disabled}
              className={cn("flex w-4 items-center justify-center rounded-r-md text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-35", tb.active && "text-selection hover:bg-selection-soft")}
            >
              <ChevronDown className="size-3" />
            </button>
          </DropdownMenuTrigger>
        </Tip>
        {menu}
      </DropdownMenu>
    </div>
  );
}

const Sep = () => <Separator orientation="vertical" className="mx-0.5 !h-9" />;

/** `compare`: another study, shown beside this one (each its largest series, scrolling together). */
export function ViewerPage({ study: studyId, series: seriesId, compare }: { study: string; series: string | null; compare?: string | null }) {
  const data = useQuery({
    queryKey: ["viewer", studyId],
    queryFn: async () => {
      const [study] = await Promise.all([api(`studies/${studyId}`), initCornerstone()]);
      return { study };
    },
  });
  const registry = useRef<Registry>(new Map()).current;
  const grid = useRef<HTMLDivElement>(null);
  const viewer = useRef<Viewer | null>(null);
  const [state, setState] = useState<ViewerState | null>(null);
  const [browser, setBrowser] = useState(true);
  const [tags, setTags] = useState(false);
  const [tagQuery, setTagQuery] = useState("");
  const [help, setHelp] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [ctx, setCtx] = useState<StudyContext | null>(null);
  const [protocols, setProtocols] = useState<Plan[]>([]);
  const [protocol, setProtocol] = useState<string | null>(null);
  const [mine, setMine] = useState<Protocol[]>([]);
  const refreshMine = () => fetchUserProtocols().then(setMine, () => setMine([]));

  // dark everywhere while the viewer is open, menus and dialogs included (they render outside)
  useEffect(() => {
    document.documentElement.classList.add("dark");
    return () => applyTheme();
  }, []);

  // the engine lives as long as the study is open; the series in the URL is the first shown
  useEffect(() => {
    if (!data.data || !grid.current) return;
    const resolve = async (id: string) => {
      const known = registry.get(id);
      if (known) return known;
      const [series, study] = await Promise.all([api(`series/${id}`), api(`series/${id}/study`)]);
      registry.set(id, { study, series });
      return { study, series };
    };
    const v = new Viewer(grid.current, setState, resolve);
    viewer.current = v;
    v.onSaveFailed = (message) => toast.error("Measurements are not being saved", { description: message });
    v.onBlank = (message) => {
      toast(message, { duration: 6000 });
      setTimeout(() => location.reload(), 1500);
    };
    // a series asked for opens on its own; a study, as its best hanging protocol lays it out
    Promise.all([studyContext(studyId), fetchUserProtocols().catch(() => [] as Protocol[])]).then(([c, saved]) => {
      if (viewer.current !== v) return;
      setCtx(c);
      setMine(saved);
      const fits = plans(c, saved);
      setProtocols(fits);
      const first = autoApply() ? fits[0] : fits.find((p) => p.protocol.id === "builtin:single");
      if (compare) {
        // each study's largest series, side by side, linked
        const largest = (id: string) =>
          api<any[]>(`studies/${id}/series`).then((list) => list.reduce((a, b) => ((b.Instances?.length ?? 0) > (a?.Instances?.length ?? -1) ? b : a), null)?.ID ?? null);
        Promise.all([largest(studyId), largest(compare)]).then(([a, b]) => {
          if (viewer.current !== v) return;
          if (a && b) v.hang(LAYOUTS.findIndex((l) => l.cols === 2 && l.rows === 1), [a, b], true);
          else v.start(a ?? c.series[0]?.ID);
        });
      } else if (seriesId || !first) {
        v.start(seriesId ?? first?.series.find(Boolean) ?? c.series[0]?.ID);
      } else {
        setProtocol(first.protocol.id);
        v.hang(first.layout, first.series, first.linked, first.mpr);
      }
    });
    return () => {
      v.destroy();
      viewer.current = null;
    };
  }, [data.data]);

  // saving or deleting a protocol changes what fits
  useEffect(() => {
    if (ctx) setProtocols(plans(ctx, mine));
  }, [ctx, mine]);

  function applyPlan(p: Plan | undefined) {
    if (!p || !viewer.current) return;
    setProtocol(p.protocol.id);
    viewer.current.hang(p.layout, p.series, p.linked, p.mpr);
  }

  // the URL follows the active viewport's study and series, without a page change
  const activeCell = state?.cells[state.active];
  // (while a protocol lays the study out, the URL names the study alone, so a reload hangs it again)
  useEffect(() => {
    // a comparison keeps its URL, so a reload compares again
    if (activeCell?.seriesId && activeCell.studyId && !compare) {
      const to = protocol ? href.view(studyId) : href.view(activeCell.studyId, activeCell.seriesId);
      if (location.hash !== to) history.replaceState(null, "", to);
    }
  }, [activeCell?.seriesId, activeCell?.studyId, protocol]);

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
      if (target.closest("input, textarea, select, [role=dialog], [role=menu]")) return;
      // shifted letters arrive in capitals: E is the rectangle, R rotates left
      const tool: Record<string, ToolName> = {
        w: "windowLevel", p: "pan", z: "zoom", m: "magnify",
        l: "length", a: "angle", c: "cobb", b: "bidirectional", e: "ellipse", E: "rectangle", d: "probe",
      };
      const step: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1, PageDown: 10, PageUp: -10 };
      if (step[e.key]) {
        e.preventDefault();
        v.step(step[e.key]);
      } else if (e.key === "Home") v.first();
      else if (e.key === "End") v.last();
      else if (e.key === "Delete" && e.shiftKey) clearMeasurements();
      else if (tool[e.key]) v.useTool(tool[e.key]);
      else if (e.key >= "1" && e.key <= String(PRESETS.length)) v.setPreset(Number(e.key) - 1);
      else if (e.key === "i") v.toggleInvert();
      else if (e.key === "r") v.rotate(90);
      else if (e.key === "R") v.rotate(-90);
      else if (e.key === "h") v.flip("horizontal");
      else if (e.key === "v") v.flip("vertical");
      else if (e.key === "0") v.reset();
      else if (e.key === "x") state?.mode === "mpr" ? v.exitMpr() : startMpr();
      else if (e.key === "n") v.addView();
      else if (e.key === " ") {
        e.preventDefault();
        v.cine();
      } else if (e.key === "s") v.setLinked(!state?.linked);
      else if (e.key === "o") v.setOverlays(!state?.overlays);
      else if (e.key === "t") setTags((x) => !x);
      else if (e.key === "f") toggleFullscreen();
      else if (e.key === "[") v.nextSeries(-1);
      else if (e.key === "]") v.nextSeries(1);
      else if (e.key === "?") setHelp(true);
      else if (e.key === "," || e.key === ".") {
        const at = protocols.findIndex((p) => p.protocol.id === protocol);
        applyPlan(protocols[at < 0 ? 0 : at + (e.key === "," ? -1 : 1)]);
      }
      else if (e.key === "Escape" && !document.fullscreenElement) location.hash = href.studies(null, studyId);
    };
    addEventListener("keydown", keys);
    return () => removeEventListener("keydown", keys);
  }, [state?.linked, state?.overlays, state?.mode, studyId, protocols, protocol]);

  const v = viewer.current;
  const s = state;
  function clearMeasurements() {
    const v = viewer.current;
    if (!v) return;
    const removed = v.clearMeasurements();
    if (removed.length)
      toast(`${removed.length} measurement${removed.length === 1 ? "" : "s"} removed`, {
        action: { label: "Undo", onClick: () => viewer.current?.restoreMeasurements(removed) },
        duration: 8000,
      });
  }
  function startMpr() {
    viewer.current?.enterMpr().then((error) => error && toast.error("Couldn't open MPR", { description: error }));
  }
  const study = data.data?.study;
  const mpr = s?.mode === "mpr";
  const measuring = MEASURE.find((m) => m.tool === s?.tool);
  const shown = useMemo(() => new Set((s?.cells ?? []).map((c) => c.seriesId).filter(Boolean) as string[]), [s?.cells]);
  const rows = useMemo(() => (s?.meta ? tagRows(s.meta, tagQuery.toLowerCase()) : []), [s?.meta, tagQuery]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-background text-foreground">
      <header className="flex min-h-14 shrink-0 flex-wrap items-center gap-0.5 border-b bg-card px-2 py-1">
        <Tip label="Back to the studies" shortcut="Esc">
          <a href={href.studies(null, studyId)} className="mr-1 flex h-12 shrink-0 items-center gap-2 rounded-md px-2 hover:bg-accent" aria-label="Back to the studies">
            <ArrowLeft className="size-4 text-muted-foreground" />
            <Logo className="size-5" />
          </a>
        </Tip>
        <ToolBtn label="Series" Icon={PanelLeft} active={browser} onClick={() => setBrowser(!browser)} />
        {s && <ProtocolMenu plans={protocols} active={protocol} onApply={applyPlan} state={s} ctx={ctx} mine={mine} onSaved={refreshMine} />}
        <Sep />

        {s && v && (
          <>
            <ToolMenu label="Window" menuLabel="Window presets" Icon={Contrast} shortcut="W" active={s.tool === "windowLevel"} onClick={() => v.useTool("windowLevel")}>
              <DropdownMenuLabel>Window presets</DropdownMenuLabel>
              {PRESETS.map((p, i) => (
                <DropdownMenuItem key={p.name} onSelect={() => v.setPreset(i)}>
                  <Check className={cn(s.preset !== i && "opacity-0")} />
                  {p.name}
                  {p.window && <span className="ml-auto text-xs text-muted-foreground tabular">{p.window.width}/{p.window.center}</span>}
                  <DropdownMenuShortcut className="ml-2">{i + 1}</DropdownMenuShortcut>
                </DropdownMenuItem>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => v.toggleInvert()}>
                <SunMoon /> Invert <Check className={cn("ml-auto", !s.invert && "opacity-0")} />
                <DropdownMenuShortcut>I</DropdownMenuShortcut>
              </DropdownMenuItem>
            </ToolMenu>
            <ToolBtn label="Pan" Icon={Move} shortcut="P" active={s.tool === "pan"} onClick={() => v.useTool("pan")} />
            <ToolBtn label="Zoom" tip="Zoom: drag up or down" Icon={ZoomIn} shortcut="Z" active={s.tool === "zoom"} onClick={() => v.useTool("zoom")} />
            <ToolBtn
              label="Magnifier"
              tip="Magnifying glass: hold the button over a spot to see it larger; the wheel changes how much"
              Icon={ScanSearch}
              shortcut="M"
              active={s.tool === "magnify"}
              onClick={() => v.useTool("magnify")}
            />
            <Sep />
            {MEASURE.map((m) => (
              <ToolBtn key={m.tool} label={m.short} tip={`${m.label}: ${m.hint}`} Icon={m.Icon} shortcut={m.key} active={s.tool === m.tool} onClick={() => v.useTool(m.tool)} />
            ))}
            <ToolBtn label="Clear" tip="Remove all measurements (they are saved with the study)" Icon={Eraser} shortcut="⇧ Del" onClick={clearMeasurements} />
            <Sep />
            <ToolMenu label="Rotate" menuLabel="Rotate and flip" tip="Rotate right 90°" Icon={RotateCw} shortcut="R" onClick={() => v.rotate(90)}>
              <DropdownMenuItem onSelect={() => v.rotate(90)}>
                <RotateCw /> Rotate right 90° <DropdownMenuShortcut>R</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.rotate(-90)}>
                <RotateCcw /> Rotate left 90° <DropdownMenuShortcut>⇧ R</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.flip("horizontal")}>
                <ArrowLeftRight /> Flip horizontal <DropdownMenuShortcut>H</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.flip("vertical")}>
                <ArrowUpDown /> Flip vertical <DropdownMenuShortcut>V</DropdownMenuShortcut>
              </DropdownMenuItem>
            </ToolMenu>
            <ToolBtn label="Reset" tip="Reset the view: fit, no rotation, default window" Icon={RotateCcw} shortcut="0" onClick={() => v.reset()} />
            <Sep />

            <ToolMenu
              label="Add view"
              menuLabel="Add a viewport: which plane or series"
              tip={s.canMpr ? "Add a viewport: this series in the next plane (sagittal, then coronal)" : "Add a viewport with the next series"}
              Icon={SquarePlus}
              shortcut="N"
              disabled={!mpr && !s.canAdd}
              onClick={() => v.addView()}
            >
              <DropdownMenuLabel>Add a viewport</DropdownMenuLabel>
              {(["axial", "sagittal", "coronal"] as const).map((plane) => (
                <DropdownMenuItem key={plane} disabled={!s.canMpr} onSelect={() => v.addView({ plane })}>
                  <Axis3d />
                  This series, {plane}
                </DropdownMenuItem>
              ))}
              <DropdownMenuItem disabled={!s.canMpr} onSelect={() => v.addView({ plane: "3d" })}>
                <Box />
                <div className="flex flex-col">
                  <span>This series, in 3D</span>
                  <span className="text-xs text-muted-foreground">Drag to turn it; bone, muscle, vessels, lung, skin, MIP</span>
                </div>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => v.addView({ nextSeries: true })}>
                <SquarePlus /> The next series of the study
              </DropdownMenuItem>
              {!s.canMpr && s.mprBlocked && <div className="px-2 pb-1.5 text-xs text-muted-foreground">{s.mprBlocked}.</div>}
            </ToolMenu>
            <ToolMenu label="Views" menuLabel="Views: how many, and which planes" Icon={LayoutGrid}>
              <DropdownMenuLabel>How many viewports</DropdownMenuLabel>
              <div className="grid grid-cols-3 gap-1 p-1">
                {LAYOUTS.map((l, i) => (
                  <DropdownMenuItem
                    key={l.name}
                    aria-label={`Layout ${l.name}`}
                    onSelect={() => v.setLayout(i)}
                    className={cn("flex flex-col items-center gap-1 rounded-md p-2 text-[11px] text-muted-foreground", !mpr && s.layout === i && "bg-selection-soft text-selection")}
                  >
                    <span className="grid h-7 w-9 gap-0.5" style={{ gridTemplateColumns: `repeat(${l.cols}, 1fr)`, gridTemplateRows: `repeat(${l.rows}, 1fr)` }}>
                      {Array.from({ length: l.cols * l.rows }, (_, j) => (
                        <span key={j} className="rounded-[2px] border border-current" />
                      ))}
                    </span>
                    {l.name}
                  </DropdownMenuItem>
                ))}
              </div>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Planes</DropdownMenuLabel>
              <DropdownMenuItem disabled={!mpr && !s.canMpr} onSelect={() => (mpr ? v.exitMpr() : startMpr())}>
                <Axis3d />
                <div className="flex flex-col">
                  <span>{mpr ? "Leave the 3 planes" : "3 planes with crosshairs"}</span>
                  <span className="text-xs text-muted-foreground">{s.mprBlocked && !mpr ? s.mprBlocked : "Axial, sagittal, coronal and a MIP of this series"}</span>
                </div>
                <DropdownMenuShortcut>X</DropdownMenuShortcut>
              </DropdownMenuItem>
              <div className="px-2 pb-1.5 text-xs text-muted-foreground">Any viewport can also show another plane, or 3D: use the plane menu at its bottom.</div>
              <DropdownMenuSeparator />
              <DropdownMenuCheckboxItem checked={s.linked} disabled={mpr || s.cells.length < 2} onSelect={(e) => e.preventDefault()} onCheckedChange={(on) => v.setLinked(on)}>
                <div className="flex flex-col">
                  <span>Scroll the viewports together</span>
                  <span className="text-xs text-muted-foreground">Each one follows to the same slice position</span>
                </div>
                <DropdownMenuShortcut>S</DropdownMenuShortcut>
              </DropdownMenuCheckboxItem>
              <DropdownMenuSeparator />
              <DropdownMenuCheckboxItem
                checked={renderingMode() === "cpu"}
                onCheckedChange={(on) => {
                  setRenderingMode(on ? "cpu" : "gpu");
                  location.reload();
                }}
              >
                <div className="flex flex-col">
                  <span>Software rendering</span>
                  <span className="text-xs text-muted-foreground">Without WebGL, if images stay blank in this browser; no 3 planes then</span>
                </div>
              </DropdownMenuCheckboxItem>
            </ToolMenu>
            <ToolBtn
              label={s.loadingVolume ? "Loading…" : "3 planes"}
              tip={!mpr && s.mprBlocked ? s.mprBlocked : "Axial, sagittal and coronal of this series, with crosshairs, and a MIP"}
              Icon={Axis3d}
              shortcut="X"
              active={mpr}
              disabled={!mpr && (!s.canMpr || s.loadingVolume)}
              onClick={() => (mpr ? v.exitMpr() : startMpr())}
            />
            {mpr && <ToolBtn label="Crosshairs" tip="Crosshairs: drag them to move through the three planes" Icon={Crosshair} active={s.tool === "crosshairs"} onClick={() => v.useTool("crosshairs")} />}
            {!mpr && s.cells.length > 1 && (
              <ToolBtn label="Sync" tip="Scroll the viewports together, to the same slice position" Icon={Link2} shortcut="S" active={s.linked} onClick={() => v.setLinked(!s.linked)} />
            )}
            <ToolBtn label={s.playing ? "Stop" : "Cine"} Icon={s.playing ? Pause : Play} shortcut="Space" active={s.playing} onClick={() => v.cine()} />
            <Sep />

            {/* on a narrower screen, the last tools go into one menu, so the toolbar keeps to one row */}
            <div className="hidden min-[1600px]:contents">
              <ToolBtn label="Tags" Icon={Tags} shortcut="T" active={tags} onClick={() => setTags(!tags)} />
              <ToolBtn label="Info" Icon={Info} shortcut="O" active={s.overlays} onClick={() => v.setOverlays(!s.overlays)} />
              <ToolMenu label="Export" menuLabel="Export" Icon={ImageDown} onClick={() => v.exportImage()}>
                <DropdownMenuItem onSelect={() => v.exportImage()}>
                  <ImageDown /> This view as PNG
                </DropdownMenuItem>
                <DropdownMenuItem asChild disabled={!s.instanceFile}>
                  <a href={s.instanceFile ?? undefined} download>
                    <Download /> This image as DICOM
                  </a>
                </DropdownMenuItem>
              </ToolMenu>
              <ToolBtn label={fullscreen ? "Exit full" : "Full screen"} Icon={fullscreen ? Minimize : Maximize} shortcut="F" onClick={toggleFullscreen} />
              <ToolBtn label="Keys" Icon={Keyboard} shortcut="?" onClick={() => setHelp(true)} />
            </div>
            <div className="min-[1600px]:hidden">
              <ToolMenu label="More" menuLabel="More: tags, text, export, full screen, keys" Icon={Ellipsis}>
                <DropdownMenuCheckboxItem checked={tags} onCheckedChange={setTags}>
                  <Tags /> DICOM tags <DropdownMenuShortcut>T</DropdownMenuShortcut>
                </DropdownMenuCheckboxItem>
                <DropdownMenuCheckboxItem checked={s.overlays} onCheckedChange={(on) => v.setOverlays(on)}>
                  <Info /> Text on the images <DropdownMenuShortcut>O</DropdownMenuShortcut>
                </DropdownMenuCheckboxItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => v.exportImage()}>
                  <ImageDown /> This view as PNG
                </DropdownMenuItem>
                <DropdownMenuItem asChild disabled={!s.instanceFile}>
                  <a href={s.instanceFile ?? undefined} download>
                    <Download /> This image as DICOM
                  </a>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={toggleFullscreen}>
                  {fullscreen ? <Minimize /> : <Maximize />} {fullscreen ? "Leave full screen" : "Full screen"} <DropdownMenuShortcut>F</DropdownMenuShortcut>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => setHelp(true)}>
                  <Keyboard /> Keyboard and mouse <DropdownMenuShortcut>?</DropdownMenuShortcut>
                </DropdownMenuItem>
              </ToolMenu>
            </div>
          </>
        )}
      </header>

      <div className="flex min-h-0 flex-1">
        {browser && study && (
          <StudyBrowser
            study={study}
            registry={registry}
            shown={shown}
            active={activeCell?.seriesId ?? null}
            onPick={(id) => {
              setProtocol(null);
              v?.showSeries(id);
            }}
          />
        )}
        <main ref={grid} className="viewer-grid min-w-0 flex-1" data-tool={s?.tool} />
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
                    <td className="py-1.5 pr-2 font-mono whitespace-nowrap text-muted-foreground" style={{ paddingLeft: 16 + r.depth * 14 }}>
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
            <DialogDescription>The keys act on the active viewport, the one framed in blue.</DialogDescription>
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
