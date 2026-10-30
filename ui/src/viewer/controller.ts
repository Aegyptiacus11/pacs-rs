// The viewer's engine: one to four Cornerstone3D stack viewports in a grid element, with no UI
// of its own beyond the text over each image. The React toolbar drives it and redraws from the
// state it reports.
//
// Frames come over WADO-RS in the transfer syntax they are stored in (JPEG-LS by default) and
// are decoded in web workers by dicom-image-loader's codecs; each series' metadata is fetched
// once and handed to the loader, so no image needs a request of its own for it.

import { init as coreInit, RenderingEngine, Enums, eventTarget, utilities, type Types } from "@cornerstonejs/core";
import { init as loaderInit, wadors } from "@cornerstonejs/dicom-image-loader";
import {
  init as toolsInit,
  addTool,
  ToolGroupManager,
  WindowLevelTool,
  PanTool,
  ZoomTool,
  StackScrollTool,
  LengthTool,
  ProbeTool,
  AngleTool,
  CobbAngleTool,
  BidirectionalTool,
  EllipticalROITool,
  RectangleROITool,
  annotation,
  Enums as ToolEnums,
  utilities as toolUtilities,
} from "@cornerstonejs/tools";
import { api, formatDate, formatTime, personName, ROOT, sexAge, WEB } from "@/lib/api";
import { first, num, tag } from "@/lib/pixels";

export const TOOLS = {
  windowLevel: WindowLevelTool,
  pan: PanTool,
  zoom: ZoomTool,
  length: LengthTool,
  probe: ProbeTool,
  angle: AngleTool,
  cobb: CobbAngleTool,
  bidirectional: BidirectionalTool,
  ellipse: EllipticalROITool,
  rectangle: RectangleROITool,
} as const;
export type ToolName = keyof typeof TOOLS;

export const PRESETS = [
  { name: "Default", window: null },
  { name: "Soft tissue", window: { center: 40, width: 400 } },
  { name: "Liver", window: { center: 60, width: 160 } },
  { name: "Lung", window: { center: -600, width: 1500 } },
  { name: "Bone", window: { center: 400, width: 1800 } },
  { name: "Brain", window: { center: 40, width: 80 } },
] as const;

export const LAYOUTS = [
  { name: "1 × 1", cols: 1, rows: 1 },
  { name: "1 × 2", cols: 2, rows: 1 },
  { name: "2 × 1", cols: 1, rows: 2 },
  { name: "2 × 2", cols: 2, rows: 2 },
] as const;

const ENGINE = "pacs-rs";
const GROUP = "pacs-rs-tools";

let ready: Promise<void> | null = null;
export function initCornerstone() {
  ready ??= (async () => {
    await coreInit();
    loaderInit({ maxWebWorkers: Math.max(1, Math.min(4, navigator.hardwareConcurrency || 1)) });
    toolsInit();
    // measurements in the selection blue, and white while being moved
    const blue = "rgb(125, 168, 255)", white = "rgb(244, 245, 247)";
    const defaults = annotation.config.style.getDefaultToolStyles();
    annotation.config.style.setDefaultToolStyles({
      ...defaults,
      global: {
        ...defaults.global,
        color: blue,
        colorHighlighted: white,
        colorSelected: white,
        textBoxColor: blue,
        textBoxColorHighlighted: white,
        textBoxColorSelected: white,
        textBoxFontFamily: '"Inter Variable", system-ui, sans-serif',
        textBoxFontSize: "12px",
        textBoxBackground: "rgba(10, 12, 16, 0.72)",
      },
    });
    for (const tool of [StackScrollTool, ...Object.values(TOOLS)]) addTool(tool);
  })();
  return ready;
}

// orientation markers: where each edge of the screen points in the patient, measured by
// projecting the patient axes (DICOM LPS) through the viewport, so rotation and flips count
const AXES: [Types.Point3, string, string][] = [
  [[1, 0, 0], "L", "R"],
  [[0, 1, 0], "P", "A"],
  [[0, 0, 1], "H", "F"],
];
function markers(viewport: Types.IStackViewport) {
  const f = viewport.getCamera().focalPoint!;
  const c = viewport.worldToCanvas(f);
  const deltas = AXES.map(([axis]) => {
    const q = viewport.worldToCanvas(f.map((x, i) => x + axis[i] * 100) as Types.Point3);
    return [q[0] - c[0], q[1] - c[1]];
  });
  const edge = (u: number[]) => {
    const scores = deltas.map((d) => d[0] * u[0] + d[1] * u[1]);
    const top = Math.max(...scores.map(Math.abs));
    return AXES.map((a, i) => [scores[i], a] as const)
      .filter(([x]) => top > 0 && Math.abs(x) > 0.25 * top)
      .sort((a, b) => Math.abs(b[0]) - Math.abs(a[0]))
      .slice(0, 2)
      .map(([x, [, plus, minus]]) => (x > 0 ? plus : minus))
      .join("");
  };
  return { or: edge([1, 0]), ol: edge([-1, 0]), ot: edge([0, -1]), ob: edge([0, 1]) };
}
const cross = (a: number[], b: number[]) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a: number[], b: number[]) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

type Slot = { imageId: string; instance: any; meta: any; frame: number; frames: number };
type Cell = {
  i: number;
  id: string;
  element: HTMLDivElement;
  wrap: HTMLDivElement;
  ov: Record<"tl" | "tr" | "bl" | "br" | "ol" | "or" | "ot" | "ob", HTMLDivElement>;
  note: HTMLDivElement;
  viewport: Types.IStackViewport;
  seriesId: string | null;
  series: any;
  slots: Slot[];
  index: number;
  preset: number;
  invert: boolean;
  rotation: number;
  fitZoom: number | null;
  failed: Set<string>;
  playing: boolean;
};

export type ViewerState = {
  layout: number;
  active: number;
  shown: (string | null)[];
  tool: ToolName;
  linked: boolean;
  overlays: boolean;
  preset: number;
  invert: boolean;
  playing: boolean;
  index: number;
  count: number;
  instanceFile: string | null;
  meta: any;
};

export class Viewer {
  private engine: RenderingEngine;
  private group: ReturnType<typeof ToolGroupManager.createToolGroup> & {};
  private cells: Cell[] = [];
  private current = 0;
  private layout = 0;
  private linked = false;
  private syncing = false;
  private overlaysOn = true;
  private tool: ToolName = "windowLevel";
  private loaded = new Map<string, Promise<{ series: any; slots: Slot[] }>>();
  private resize: ResizeObserver;
  private onLoadFailed = (e: any) => {
    const id = e.detail?.imageId;
    for (const cell of this.cells)
      if (cell.slots.some((s) => s.imageId === id)) {
        cell.failed.add(id);
        this.overlays(cell);
      }
  };

  constructor(
    private grid: HTMLDivElement,
    private study: any,
    private allSeries: any[],
    private onChange: (state: ViewerState) => void,
  ) {
    ToolGroupManager.destroyToolGroup(GROUP);
    this.engine = new RenderingEngine(ENGINE);
    this.group = ToolGroupManager.createToolGroup(GROUP)!;
    for (const tool of [StackScrollTool, ...Object.values(TOOLS)]) this.group.addTool(tool.toolName);
    this.group.setToolActive(StackScrollTool.toolName, { bindings: [{ mouseButton: ToolEnums.MouseBindings.Wheel }] });
    this.applyTool(null, "windowLevel");
    // measurements belong to this viewing only
    annotation.state.removeAllAnnotations();
    eventTarget.addEventListener(Enums.Events.IMAGE_LOAD_FAILED, this.onLoadFailed);
    this.resize = new ResizeObserver(() => this.refit());
    this.resize.observe(grid);
  }

  async start(seriesId: string) {
    this.setGrid(1, 1);
    this.cells.push(this.makeCell(0));
    this.refit();
    await this.show(this.cells[0], seriesId);
    this.cells[0].wrap.focus();
  }

  destroy() {
    eventTarget.removeEventListener(Enums.Events.IMAGE_LOAD_FAILED, this.onLoadFailed);
    this.resize.disconnect();
    for (const cell of this.cells) {
      if (cell.playing) this.cine(false, cell);
      try {
        toolUtilities.stackPrefetch.disable(cell.element);
      } catch {}
    }
    ToolGroupManager.destroyToolGroup(GROUP);
    this.engine.destroy();
  }

  // ---- state for the toolbar ----

  private emit() {
    const cell = this.cells[this.current];
    const slot = cell?.slots[cell.index];
    this.onChange({
      layout: this.layout,
      active: this.current,
      shown: this.cells.map((c) => c.seriesId),
      tool: this.tool,
      linked: this.linked,
      overlays: this.overlaysOn,
      preset: cell?.preset ?? 0,
      invert: cell?.invert ?? false,
      playing: cell?.playing ?? false,
      index: cell?.index ?? 0,
      count: cell?.slots.length ?? 0,
      instanceFile: slot ? new URL(`instances/${slot.instance.ID}/file`, ROOT).href : null,
      meta: slot?.meta ?? null,
    });
  }

  // ---- series and cells ----

  private loadSeries(id: string) {
    if (!this.loaded.has(id)) {
      this.loaded.set(
        id,
        (async () => {
          const series = this.allSeries.find((s) => s.ID === id);
          const base = new URL(`studies/${this.study.MainDicomTags.StudyInstanceUID}/series/${series.MainDicomTags.SeriesInstanceUID}/`, WEB);
          const [instances, metadata] = await Promise.all([api<any[]>(`series/${id}/instances`), api<any[]>("metadata", {}, base)]);
          const byUid = new Map(metadata.map((m) => [first(m, "00080018"), m]));
          const slots: Slot[] = [];
          for (const instance of instances) {
            const uid = instance.MainDicomTags.SOPInstanceUID;
            const meta = byUid.get(uid);
            if (!meta) continue;
            const frames = Math.max(1, num(meta, "00280008", 1));
            for (let f = 1; f <= frames; f++) {
              const imageId = `wadors:${base}instances/${uid}/frames/${f}`;
              wadors.metaDataManager.add(imageId, meta);
              slots.push({ imageId, instance, meta, frame: f, frames });
            }
          }
          return { series, slots };
        })(),
      );
    }
    return this.loaded.get(id)!;
  }

  private makeCell(i: number): Cell {
    const div = (cls: string) => Object.assign(document.createElement("div"), { className: cls });
    const element = div("cs absolute inset-0") as HTMLDivElement;
    const ov = Object.fromEntries((["tl", "tr", "bl", "br", "ol", "or", "ot", "ob"] as const).map((k) => [k, div(`ov ov-${k}`)])) as Cell["ov"];
    const note = div("ov-note");
    const wrap = div("cell") as HTMLDivElement;
    wrap.tabIndex = 0;
    wrap.append(element, ...Object.values(ov), note);
    this.grid.append(wrap);
    const id = `vp${i}`;
    this.engine.enableElement({ viewportId: id, type: Enums.ViewportType.STACK, element, defaultOptions: { background: [0, 0, 0] } });
    this.group.addViewport(id, ENGINE);
    const cell: Cell = {
      i, id, element, wrap, ov, note,
      viewport: this.engine.getViewport(id) as Types.IStackViewport,
      seriesId: null, series: null, slots: [], index: 0, preset: 0, invert: false, rotation: 0, fitZoom: null, failed: new Set(), playing: false,
    };
    wrap.addEventListener("pointerdown", () => this.activate(cell.i), true);
    wrap.addEventListener("contextmenu", (e) => e.preventDefault());
    wrap.addEventListener("dblclick", () => this.reset(cell));
    element.addEventListener(Enums.Events.STACK_NEW_IMAGE, ((e: any) => {
      cell.index = e.detail.imageIdIndex;
      this.overlays(cell);
      if (this.linked && !this.syncing && cell === this.active()) this.syncFrom(cell);
      if (cell === this.active()) this.emit();
    }) as EventListener);
    element.addEventListener(Enums.Events.VOI_MODIFIED, () => {
      const w = PRESETS[cell.preset]?.window;
      if (w) {
        const voi = cell.viewport.getProperties()?.voiRange;
        const want = utilities.windowLevel.toLowHighRange(w.width, w.center);
        if (voi && (Math.abs(voi.lower - want.lower) > 0.5 || Math.abs(voi.upper - want.upper) > 0.5)) {
          cell.preset = -1;
          if (cell === this.active()) this.emit();
        }
      }
      this.overlays(cell);
    });
    element.addEventListener(Enums.Events.CAMERA_MODIFIED, () => this.overlays(cell));
    return cell;
  }

  private async show(cell: Cell, id: string, at: number | null = null) {
    if (cell.playing) this.cine(false, cell);
    cell.seriesId = id;
    cell.note.textContent = "Loading…";
    this.emit();
    const { series, slots } = await this.loadSeries(id);
    if (cell.seriesId !== id) return;
    Object.assign(cell, { series, slots, failed: new Set(), preset: 0, rotation: 0 });
    if (!slots.length) {
      cell.note.textContent = "This series has no images";
      return this.emit();
    }
    cell.index = at ?? Math.floor(slots.length / 2);
    try {
      await cell.viewport.setStack(
        slots.map((s) => s.imageId),
        cell.index,
      );
      if (cell.invert) cell.viewport.setProperties({ invert: true });
    } catch {
      cell.failed.add(slots[cell.index].imageId);
    }
    cell.viewport.render();
    try {
      cell.fitZoom = cell.viewport.getZoom();
    } catch {}
    toolUtilities.stackPrefetch.enable(cell.element);
    this.overlays(cell);
    this.emit();
  }

  private overlays(cell: Cell) {
    const { ov, slots, series } = cell;
    if (!slots.length || !series) return;
    const p = this.study.PatientMainDicomTags, t = this.study.MainDicomTags, st = series.MainDicomTags;
    const slot = slots[cell.index];
    const pos = tag(slot.meta, "00200032");
    const thickness = num(slot.meta, "00180050", NaN);
    const columns = num(slot.meta, "00280011", 0), rows = num(slot.meta, "00280010", 0);
    ov.tl.textContent = `${personName(p.PatientName)}\n${[p.PatientID, sexAge(p.PatientSex, p.PatientBirthDate, t.StudyDate)].filter(Boolean).join("  ·  ")}\n${formatDate(t.StudyDate)} ${formatTime(t.StudyTime)}`;
    ov.tr.textContent = `${t.StudyDescription || ""}\n${st.Modality || ""} ${st.SeriesNumber ?? ""}  ${st.SeriesDescription || ""}`;
    ov.br.textContent = [pos ? `Z ${Number(pos[2]).toFixed(1)} mm` : null, isFinite(thickness) ? `Thickness ${thickness} mm` : null, rows ? `${columns} × ${rows}` : null].filter(Boolean).join("\n");
    const voi = cell.viewport.getProperties()?.voiRange;
    let zoom = 1;
    try {
      if (cell.fitZoom) zoom = cell.viewport.getZoom() / cell.fitZoom;
    } catch {}
    const wl = voi ? utilities.windowLevel.toWindowLevel(voi.lower, voi.upper) : null;
    ov.bl.textContent = [
      `Image ${cell.index + 1} / ${slots.length}${slot.frames > 1 ? `  ·  frame ${slot.frame}` : ""}`,
      wl ? `W ${Math.round(wl.windowWidth)}  L ${Math.round(wl.windowCenter)}` : "",
      Math.abs(zoom - 1) > 0.01 ? `Zoom ${Math.round(zoom * 100)}%` : "",
    ].filter(Boolean).join("\n");
    try {
      for (const [k, v] of Object.entries(markers(cell.viewport))) ov[k as keyof Cell["ov"]].textContent = v;
    } catch {}
    cell.note.textContent = cell.failed.has(slot.imageId) ? "This image could not be decoded" : "";
  }

  private active() {
    return this.cells[this.current];
  }

  private activate(i: number) {
    if (i === this.current) return;
    this.current = i;
    this.frame();
    this.emit();
  }

  private frame() {
    this.cells.forEach((c) => c.wrap.classList.toggle("active", c.i === this.current && this.cells.length > 1));
  }

  private setGrid(cols: number, rows: number) {
    this.grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
    this.grid.style.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`;
  }

  // a resize fits each image again: zoom is shown relative to that fit
  private refit() {
    this.engine.resize(true, false);
    for (const cell of this.cells) {
      try {
        cell.fitZoom = cell.viewport.getZoom();
      } catch {}
      this.overlays(cell);
    }
  }

  // linked scrolling: the slice of each other viewport nearest to this one's, along its normal
  private syncFrom(source: Cell) {
    if (!source.slots.length) return;
    const slot = source.slots[source.index];
    const frameOfReference = first(slot.meta, "00200052", null);
    const pos = tag(slot.meta, "00200032");
    if (!pos) return;
    this.syncing = true;
    for (const cell of this.cells) {
      if (cell === source || !cell.slots.length) continue;
      if (frameOfReference && first(cell.slots[0].meta, "00200052", null) !== frameOfReference) continue;
      const iop = (tag(cell.slots[0].meta, "00200037") || [1, 0, 0, 0, 1, 0]).map(Number);
      const normal = cross(iop.slice(0, 3), iop.slice(3, 6));
      const target = dot(pos.map(Number), normal);
      let best = cell.index, bestDistance = Infinity;
      cell.slots.forEach((s, j) => {
        const q = tag(s.meta, "00200032");
        if (!q) return;
        const d = Math.abs(dot(q.map(Number), normal) - target);
        if (d < bestDistance) [best, bestDistance] = [j, d];
      });
      if (best !== cell.index) cell.viewport.setImageIdIndex(best);
    }
    this.syncing = false;
  }

  // the left button runs the chosen tool; pan stays on the middle button and shift-drag,
  // zoom on the right button, whatever is chosen
  private bindings(name: ToolName, left: boolean) {
    const { MouseBindings, KeyboardBindings } = ToolEnums;
    const b: any[] = [];
    if (left) b.push({ mouseButton: MouseBindings.Primary });
    if (name === "pan") b.push({ mouseButton: MouseBindings.Auxiliary }, { mouseButton: MouseBindings.Primary, modifierKey: KeyboardBindings.Shift });
    if (name === "zoom") b.push({ mouseButton: MouseBindings.Secondary });
    return b;
  }
  private applyTool(from: ToolName | null, to: ToolName) {
    for (const name of Object.keys(TOOLS) as ToolName[]) {
      if (from && name !== from && name !== to && name !== "pan" && name !== "zoom") continue;
      const b = this.bindings(name, name === to);
      if (b.length) this.group.setToolActive(TOOLS[name].toolName, { bindings: b });
      else this.group.setToolPassive(TOOLS[name].toolName);
    }
  }

  // ---- what the toolbar calls ----

  useTool(name: ToolName) {
    if (name === this.tool) return;
    this.applyTool(this.tool, name);
    this.tool = name;
    this.grid.dataset.tool = name;
    this.emit();
  }

  async setLayout(i: number) {
    const { cols, rows } = LAYOUTS[i];
    this.layout = i;
    this.setGrid(cols, rows);
    const n = cols * rows;
    while (this.cells.length > n) {
      const cell = this.cells.pop()!;
      if (cell.playing) this.cine(false, cell);
      try {
        toolUtilities.stackPrefetch.disable(cell.element);
      } catch {}
      this.group.removeViewports(ENGINE, cell.id);
      this.engine.disableElement(cell.id);
      cell.wrap.remove();
    }
    const fresh: Cell[] = [];
    while (this.cells.length < n) {
      const cell = this.makeCell(this.cells.length);
      this.cells.push(cell);
      fresh.push(cell);
    }
    this.refit();
    if (this.current >= this.cells.length) this.current = 0;
    this.frame();
    // new viewports show the series after those already shown
    const start = Math.max(0, this.allSeries.findIndex((s) => s.ID === this.cells[0].seriesId));
    await Promise.all(fresh.map((cell) => this.show(cell, this.allSeries[(start + cell.i) % this.allSeries.length].ID)));
    if (this.linked) this.syncFrom(this.active());
    this.emit();
  }

  showSeries(id: string) {
    return this.show(this.active(), id);
  }
  nextSeries(step: number) {
    const at = this.allSeries.findIndex((s) => s.ID === this.active().seriesId);
    const next = this.allSeries[at + step];
    if (next) return this.showSeries(next.ID);
  }
  activeSeries() {
    return this.active()?.seriesId ?? null;
  }

  go(i: number) {
    const cell = this.active();
    const n = Math.max(0, Math.min(cell.slots.length - 1, i));
    if (cell.slots.length && n !== cell.viewport.getCurrentImageIdIndex()) cell.viewport.setImageIdIndex(n);
  }
  step(n: number) {
    this.go(this.active().index + n);
  }
  first() {
    this.go(0);
  }
  last() {
    this.go(this.active().slots.length - 1);
  }

  setPreset(i: number, cell = this.active()) {
    cell.preset = i;
    const w = PRESETS[i].window;
    if (w) cell.viewport.setProperties({ voiRange: utilities.windowLevel.toLowHighRange(w.width, w.center) });
    else {
      cell.viewport.resetProperties();
      cell.viewport.setProperties({ invert: cell.invert });
    }
    cell.viewport.render();
    this.emit();
  }
  toggleInvert(cell = this.active()) {
    cell.invert = !cell.invert;
    cell.viewport.setProperties({ invert: cell.invert });
    cell.viewport.render();
    this.emit();
  }
  rotate(cell = this.active()) {
    cell.rotation = (cell.rotation + 90) % 360;
    cell.viewport.setViewPresentation({ rotation: cell.rotation });
    cell.viewport.render();
    this.overlays(cell);
  }
  // as seen on screen: on an image turned a quarter, left-right is the image's up-down
  flip(which: "horizontal" | "vertical", cell = this.active()) {
    let key: "flipHorizontal" | "flipVertical" = which === "horizontal" ? "flipHorizontal" : "flipVertical";
    if (cell.rotation % 180 === 90) key = key === "flipHorizontal" ? "flipVertical" : "flipHorizontal";
    cell.viewport.setCamera({ [key]: !cell.viewport.getCamera()[key] });
    cell.viewport.render();
    this.overlays(cell);
  }
  reset(cell = this.active()) {
    cell.viewport.resetCamera();
    cell.rotation = 0;
    try {
      cell.fitZoom = cell.viewport.getZoom();
    } catch {}
    this.setPreset(0, cell);
  }
  setLinked(on: boolean) {
    this.linked = on;
    if (on) this.syncFrom(this.active());
    this.emit();
  }
  setOverlays(on: boolean) {
    this.overlaysOn = on;
    this.grid.classList.toggle("no-overlays", !on);
    this.emit();
  }
  clearMeasurements() {
    annotation.state.removeAllAnnotations();
    this.cells.forEach((c) => c.viewport.render());
  }
  cine(on?: boolean, cell = this.active()) {
    on ??= !cell.playing;
    if (cell.slots.length < 2) return;
    cell.playing = on;
    if (on) toolUtilities.cine.playClip(cell.element, { framesPerSecond: 15, loop: true });
    else toolUtilities.cine.stopClip(cell.element);
    this.emit();
  }
}
