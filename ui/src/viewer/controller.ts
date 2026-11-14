// The viewer's engine: a grid of Cornerstone3D viewports with no UI of its own beyond what sits
// on each image (text, orientation letters, the position bar, the preset and slab menus, the
// scale). The React toolbar drives it and redraws from the state it reports.
//
// Two modes. Stack: one to four viewports, each showing any series of the patient (priors
// included). MPR: the series as a volume, axial, sagittal and coronal with crosshairs, and a
// fourth view, a full-thickness coronal MIP.
//
// Measurements are kept on the server per study: they come back when a study is opened again.
//
// Frames come over WADO-RS in the transfer syntax they are stored in (JPEG-LS by default) and
// are decoded in web workers by dicom-image-loader's codecs; each series' metadata is fetched
// once and handed to the loader, so no image needs a request of its own for it.

import {
  init as coreInit,
  getShouldUseCPURendering,
  setUseCPURendering,
  RenderingEngine,
  Enums,
  eventTarget,
  utilities,
  volumeLoader,
  cornerstoneStreamingImageVolumeLoader,
  setVolumesForViewports,
  cache,
  type Types,
} from "@cornerstonejs/core";
import { init as loaderInit, wadors } from "@cornerstonejs/dicom-image-loader";
import {
  init as toolsInit,
  addTool,
  ToolGroupManager,
  WindowLevelTool,
  PanTool,
  ZoomTool,
  StackScrollTool,
  MagnifyTool,
  CrosshairsTool,
  TrackballRotateTool,
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
import { api, formatDate, formatTime, personName, sexAge, WEB } from "@/lib/api";
import { first, num, tag } from "@/lib/pixels";
import { authHeaders } from "@/lib/session";
import { fetchAnnotations, saveAnnotations } from "@/viewer/annotations";

export const TOOLS = {
  windowLevel: WindowLevelTool,
  pan: PanTool,
  zoom: ZoomTool,
  magnify: MagnifyTool,
  crosshairs: CrosshairsTool,
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

/** Slabs of an MPR view: thin, or a maximum intensity projection this thick (mm). */
export const SLABS = [
  { name: "Thin", mm: 0 },
  { name: "MIP 5 mm", mm: 5 },
  { name: "MIP 10 mm", mm: 10 },
  { name: "MIP 20 mm", mm: 20 },
  { name: "MIP full", mm: 10_000 },
] as const;

/** The plane a viewport shows: the images as acquired, one of the volume's, or the volume
 * rendered in 3D. */
export const PLANES = [
  { plane: "original", name: "Original" },
  { plane: "axial", name: "Axial" },
  { plane: "sagittal", name: "Sagittal" },
  { plane: "coronal", name: "Coronal" },
  { plane: "3d", name: "3D" },
] as const;
export type Plane = (typeof PLANES)[number]["plane"];

/** How a 3D view renders the volume: Cornerstone's transfer functions, by modality. */
export const RENDERS = {
  CT: [
    { name: "Bone", preset: "CT-Bones" },
    { name: "Muscle and bone", preset: "CT-Muscle" },
    { name: "Vessels", preset: "CT-Chest-Contrast-Enhanced" },
    { name: "Lung", preset: "CT-Lung" },
    { name: "Skin", preset: "CT-Soft-Tissue" },
    { name: "MIP", preset: "CT-MIP" },
  ],
  MR: [
    { name: "Default", preset: "MR-Default" },
    { name: "Angio", preset: "MR-Angio" },
    { name: "T2 brain", preset: "MR-T2-Brain" },
    { name: "MIP", preset: "MR-MIP" },
  ],
} as const;
const rendersOf = (data: Loaded | null) => (data?.series.MainDicomTags.Modality === "CT" ? RENDERS.CT : RENDERS.MR);

export const LAYOUTS = [
  { name: "1 × 1", cols: 1, rows: 1 },
  { name: "1 × 2", cols: 2, rows: 1 },
  { name: "2 × 1", cols: 1, rows: 2 },
  { name: "2 × 2", cols: 2, rows: 2 },
  { name: "1 × 3", cols: 3, rows: 1 },
  { name: "2 × 3", cols: 3, rows: 2 },
] as const;

const ENGINE = "pacs-rs";
const STACK_GROUP = "pacs-rs-stack";
const MPR_GROUP = "pacs-rs-mpr";
const VR_GROUP = "pacs-rs-3d";
const PLANE_COLOR: Record<string, string> = { axial: "rgb(232, 84, 84)", sagittal: "rgb(92, 196, 120)", coronal: "rgb(96, 150, 255)" };

// How images are drawn. "gpu": WebGL, the image copied to the page as usual. "readback":
// WebGL, the image read out of WebGL and written into the page's canvas, for browsers where
// the usual copy draws nothing (Chrome with ANGLE on Vulkan and Mesa RADV: "invalid mailbox
// name", "texture is not a shared image"). "cpu": no WebGL at all, so no MPR. Kept in this
// browser; the viewer moves one step down by itself when an image comes out blank.
export type RenderingMode = "gpu" | "readback" | "cpu";
const RENDER_KEY = "pacs-rs-rendering-mode";
export function renderingMode(): RenderingMode {
  try {
    const m = localStorage.getItem(RENDER_KEY);
    return m === "readback" || m === "cpu" ? m : "gpu";
  } catch {
    return "gpu";
  }
}
export function setRenderingMode(mode: RenderingMode) {
  try {
    if (mode === "gpu") localStorage.removeItem(RENDER_KEY);
    else localStorage.setItem(RENDER_KEY, mode);
    // the first version's flag, which went straight to the CPU
    localStorage.removeItem("pacs-rs-rendering");
  } catch {}
}
try {
  localStorage.removeItem("pacs-rs-rendering");
} catch {}

// Which canvases have a WebGL context: those are the ones Cornerstone renders on, off screen.
const webgl = new WeakMap<HTMLCanvasElement, WebGLRenderingContext | WebGL2RenderingContext>();
const getContext = HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, options?: any) {
  // the page's own canvases keep their pixels in memory: the blank check reads them
  if (type === "2d" && this.closest(".viewer-grid")) options = { ...options, willReadFrequently: true };
  const context = (getContext as any).call(this, type, options);
  if (context && (type === "webgl" || type === "webgl2")) webgl.set(this, context);
  return context;
} as any;

// "readback": a WebGL canvas drawn into a 2D one is read with readPixels (right after
// rendering, while its buffer still holds the image) and put in with putImageData.
const drawImage = CanvasRenderingContext2D.prototype.drawImage;
CanvasRenderingContext2D.prototype.drawImage = function (this: CanvasRenderingContext2D, image: any, ...args: number[]) {
  const gl = renderingMode() === "readback" && image instanceof HTMLCanvasElement ? webgl.get(image) : undefined;
  if (!gl) return (drawImage as any).call(this, image, ...args);
  let [sx, sy, sw, sh, dx, dy, dw, dh] = args.length >= 8 ? args : [0, 0, image.width, image.height, ...(args.length === 4 ? args : [args[0], args[1], image.width, image.height])];
  sx = Math.round(sx);
  sy = Math.round(sy);
  sw = Math.round(sw);
  sh = Math.round(sh);
  if (sw <= 0 || sh <= 0) return;
  const pixels = new Uint8ClampedArray(sw * sh * 4);
  gl.readPixels(sx, image.height - sy - sh, sw, sh, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  // WebGL rows go bottom up
  const out = new ImageData(sw, sh);
  const row = sw * 4;
  for (let y = 0; y < sh; y++) out.data.set(pixels.subarray((sh - 1 - y) * row, (sh - y) * row), y * row);
  if (Math.round(dw) === sw && Math.round(dh) === sh) this.putImageData(out, Math.round(dx), Math.round(dy));
  else {
    const scratch = new OffscreenCanvas(sw, sh);
    scratch.getContext("2d")!.putImageData(out, 0, 0);
    (drawImage as any).call(this, scratch, 0, 0, sw, sh, dx, dy, dw, dh);
  }
} as any;

let ready: Promise<void> | null = null;
export function initCornerstone() {
  ready ??= (async () => {
    await coreInit();
    // the GPU whenever there is WebGL2, even a weak or software one: MPR needs it, and
    // Cornerstone's own check would fall back to the CPU on such machines; the CPU when this
    // browser was told to (or found WebGL drawing blank)
    try {
      if (renderingMode() === "cpu") setUseCPURendering(true);
      else if (document.createElement("canvas").getContext("webgl2")) setUseCPURendering(false);
    } catch {}
    loaderInit({
      maxWebWorkers: Math.max(1, Math.min(4, navigator.hardwareConcurrency || 1)),
      // every frame request carries the session's token, when the server asks for a login
      beforeSend: async () => authHeaders(),
    });
    volumeLoader.registerUnknownVolumeLoader(cornerstoneStreamingImageVolumeLoader as any);
    volumeLoader.registerVolumeLoader("cornerstoneStreamingImageVolume", cornerstoneStreamingImageVolumeLoader as any);
    toolsInit();
    // measurements in amber, white while being moved: readable on any image
    const amber = "rgb(255, 196, 70)", white = "rgb(244, 245, 247)";
    const defaults = annotation.config.style.getDefaultToolStyles();
    annotation.config.style.setDefaultToolStyles({
      ...defaults,
      global: {
        ...defaults.global,
        color: amber,
        colorHighlighted: white,
        colorSelected: white,
        textBoxColor: amber,
        textBoxColorHighlighted: white,
        textBoxColorSelected: white,
        textBoxFontFamily: '"Inter Variable", system-ui, sans-serif',
        textBoxFontSize: "12px",
        textBoxBackground: "rgba(0, 0, 0, 0.65)",
      },
    });
    for (const tool of [StackScrollTool, TrackballRotateTool, ...Object.values(TOOLS)]) addTool(tool);
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
function markers(viewport: Types.IViewport) {
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
type Loaded = { study: any; series: any; slots: Slot[] };
type Orientation = "axial" | "sagittal" | "coronal";
type Cell = {
  i: number;
  id: string;
  /** "3d": the volume rendered, turned with the mouse */
  kind: "stack" | "mpr" | "3d";
  orientation: Orientation | null;
  element: HTMLDivElement;
  wrap: HTMLDivElement;
  ov: Record<"tl" | "tr" | "bl" | "br" | "ol" | "or" | "ot" | "ob", HTMLDivElement>;
  note: HTMLDivElement;
  bar: HTMLDivElement;
  dot: HTMLDivElement;
  scale: HTMLDivElement;
  presetSelect: HTMLSelectElement;
  slabSelect: HTMLSelectElement;
  planeSelect: HTMLSelectElement;
  /** the tool group: "mpr" for the three crosshair views, "3d" for a 3D view, "stack" for
   * every other viewport, planes of a volume included */
  group: "stack" | "mpr" | "3d";
  viewport: Types.IViewport;
  seriesId: string | null;
  data: Loaded | null;
  index: number;
  count: number;
  preset: number;
  slab: number;
  invert: boolean;
  rotation: number;
  fitZoom: number | null;
  failed: Set<string>;
  playing: boolean;
};

export type CellState = { seriesId: string | null; studyId: string | null; kind: Cell["kind"]; orientation: Orientation | null };
const planeOf = (c: Cell): Plane => (c.kind === "3d" ? "3d" : (c.orientation ?? "original"));
export type ViewerState = {
  mode: "stack" | "mpr";
  layout: number;
  active: number;
  cells: CellState[];
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
  canMpr: boolean;
  canAdd: boolean;
  /** why MPR can't run here, when it can't */
  mprBlocked: string | null;
  loadingVolume: boolean;
};

export class Viewer {
  private engine: RenderingEngine;
  private stackGroup: NonNullable<ReturnType<typeof ToolGroupManager.createToolGroup>>;
  private mprGroup: NonNullable<ReturnType<typeof ToolGroupManager.createToolGroup>>;
  private vrGroup: NonNullable<ReturnType<typeof ToolGroupManager.createToolGroup>>;
  private cells: Cell[] = [];
  private current = 0;
  private layout = 0;
  private mode: "stack" | "mpr" = "stack";
  private linked = false;
  private syncing = false;
  private overlaysOn = true;
  private tool: ToolName = "windowLevel";
  private loadingVolume = false;
  private stackLayout = 0;
  private stackSeries: string[] = [];
  private loaded = new Map<string, Promise<Loaded>>();
  private resize: ResizeObserver;
  // measurements: the study each belongs to, the studies whose ones are loaded, saves waiting
  private studyOfAnnotation = new Map<string, string>();
  private annotationsOf = new Map<string, Promise<void>>();
  private annotationsIn = new Set<string>();
  private closed = false;
  private saves = new Map<string, ReturnType<typeof setTimeout>>();
  /** Called once when measurements could not be saved. */
  onSaveFailed: ((message: string) => void) | null = null;
  private saveFailed = false;
  private onAnnotation = (e: any) => {
    const a = e.detail?.annotation;
    // statistics worked out again on drawing are no change
    if (!a?.annotationUID || e.detail.changeType === ToolEnums.ChangeTypes.StatsUpdated) return;
    const study = this.studyOf(a);
    if (study) this.saveSoon(study);
  };
  private onLoadFailed = (e: any) => {
    const id = e.detail?.imageId;
    for (const cell of this.cells)
      if (cell.data?.slots.some((s) => s.imageId === id)) {
        cell.failed.add(id);
        this.overlays(cell);
      }
  };

  constructor(
    private grid: HTMLDivElement,
    private onChange: (state: ViewerState) => void,
    /** where a series of the patient comes from: its study and REST series object */
    private resolve: (seriesId: string) => Promise<{ study: any; series: any }>,
  ) {
    for (const g of [STACK_GROUP, MPR_GROUP, VR_GROUP]) ToolGroupManager.destroyToolGroup(g);
    this.engine = new RenderingEngine(ENGINE);
    this.stackGroup = ToolGroupManager.createToolGroup(STACK_GROUP)!;
    this.mprGroup = ToolGroupManager.createToolGroup(MPR_GROUP)!;
    for (const group of [this.stackGroup, this.mprGroup]) {
      for (const tool of [StackScrollTool, ...Object.values(TOOLS)]) {
        if (tool === CrosshairsTool && group === this.stackGroup) continue;
        if (tool === CrosshairsTool)
          group.addTool(tool.toolName, {
            getReferenceLineColor: (viewportId: string) => PLANE_COLOR[this.cells.find((c) => c.id === viewportId)?.orientation ?? "axial"],
            getReferenceLineControllable: () => true,
            getReferenceLineDraggableRotatable: () => true,
            getReferenceLineSlabThicknessControlsOn: () => false,
          });
        else group.addTool(tool.toolName);
      }
      group.setToolActive(StackScrollTool.toolName, { bindings: [{ mouseButton: ToolEnums.MouseBindings.Wheel }] });
    }
    // a 3D view has its own mouse, whatever tool is chosen: left turns it, middle or
    // shift-left pans, right or the wheel zooms
    const { MouseBindings, KeyboardBindings } = ToolEnums;
    this.vrGroup = ToolGroupManager.createToolGroup(VR_GROUP)!;
    for (const tool of [TrackballRotateTool, PanTool, ZoomTool]) this.vrGroup.addTool(tool.toolName);
    this.vrGroup.setToolActive(TrackballRotateTool.toolName, { bindings: [{ mouseButton: MouseBindings.Primary }] });
    this.vrGroup.setToolActive(PanTool.toolName, {
      bindings: [{ mouseButton: MouseBindings.Auxiliary }, { mouseButton: MouseBindings.Primary, modifierKey: KeyboardBindings.Shift }],
    });
    this.vrGroup.setToolActive(ZoomTool.toolName, { bindings: [{ mouseButton: MouseBindings.Secondary }, { mouseButton: MouseBindings.Wheel }] });
    this.applyTool(null, "windowLevel");
    // what another viewing left; this one's come from the server, study by study
    annotation.state.removeAllAnnotations();
    eventTarget.addEventListener(Enums.Events.IMAGE_LOAD_FAILED, this.onLoadFailed);
    for (const event of [ToolEnums.Events.ANNOTATION_COMPLETED, ToolEnums.Events.ANNOTATION_MODIFIED, ToolEnums.Events.ANNOTATION_REMOVED])
      eventTarget.addEventListener(event, this.onAnnotation);
    this.resize = new ResizeObserver(() => this.refit());
    this.resize.observe(grid);
  }

  async start(seriesId: string) {
    this.setGrid(1, 1);
    this.cells.push(this.makeCell(0, "stack", null));
    this.refit();
    await this.show(this.cells[0], seriesId);
    this.cells[0].wrap.focus();
  }

  destroy() {
    eventTarget.removeEventListener(Enums.Events.IMAGE_LOAD_FAILED, this.onLoadFailed);
    for (const event of [ToolEnums.Events.ANNOTATION_COMPLETED, ToolEnums.Events.ANNOTATION_MODIFIED, ToolEnums.Events.ANNOTATION_REMOVED])
      eventTarget.removeEventListener(event, this.onAnnotation);
    // what is waiting goes now
    for (const [study, timer] of this.saves) {
      clearTimeout(timer);
      this.save(study);
    }
    this.closed = true;
    this.resize.disconnect();
    for (const cell of this.cells) this.dropCell(cell, false);
    for (const g of [STACK_GROUP, MPR_GROUP, VR_GROUP]) ToolGroupManager.destroyToolGroup(g);
    this.engine.destroy();
  }

  // ---- measurements on the server ----

  /** The study a measurement belongs to: known for those loaded or drawn here, else read from
   * the image or volume it was drawn on, else the active viewport's. */
  private studyOf(a: any): string | null {
    const known = this.studyOfAnnotation.get(a.annotationUID);
    if (known) return known;
    const m = a.metadata ?? {};
    let study: string | null = (m.referencedImageId ?? "").match(/\/studies\/([0-9.]+)\/series\//)?.[1] ?? null;
    if (!study && typeof m.volumeId === "string")
      study = this.cells.find((c) => m.volumeId.endsWith(`:${c.data?.series.ID}`))?.data?.study.MainDicomTags.StudyInstanceUID ?? null;
    study ??= this.active()?.data?.study.MainDicomTags.StudyInstanceUID ?? null;
    if (study) this.studyOfAnnotation.set(a.annotationUID, study);
    return study;
  }

  private loadAnnotations(study: string) {
    if (!study || this.annotationsOf.has(study)) return;
    this.annotationsOf.set(
      study,
      fetchAnnotations(study)
        .then((list) => {
          for (const a of list) {
            if (annotation.state.getAnnotation(a.annotationUID)) continue;
            this.studyOfAnnotation.set(a.annotationUID, study);
            a.invalidated = true;
            annotation.state.addAnnotation(a, undefined as any);
          }
          this.annotationsIn.add(study);
          if (list.length) this.engine.render();
        })
        .catch(() => {}),
    );
  }

  private saveSoon(study: string) {
    clearTimeout(this.saves.get(study));
    this.saves.set(
      study,
      setTimeout(() => this.save(study), 700),
    );
  }

  private async save(study: string) {
    this.saves.delete(study);
    // never over what the server has before it has been read
    if (!this.annotationsIn.has(study)) {
      await this.annotationsOf.get(study);
      if (this.closed || !this.annotationsIn.has(study)) return;
    }
    // read now, not after a wait: once the viewer is closed the next one clears them
    const list = annotation.state.getAllAnnotations().filter((a: any) => this.studyOf(a) === study);
    try {
      await saveAnnotations(study, list);
    } catch (e: any) {
      if (!this.saveFailed) this.onSaveFailed?.(e?.message || "The server did not answer");
      this.saveFailed = true;
    }
  }

  // ---- state for the toolbar ----

  private emit() {
    const cell = this.cells[this.current];
    const slot = cell?.kind === "stack" ? cell.data?.slots[cell.index] : cell?.data?.slots[Math.floor((cell?.data?.slots.length ?? 1) / 2)];
    this.onChange({
      mode: this.mode,
      layout: this.layout,
      active: this.current,
      cells: this.cells.map((c) => ({ seriesId: c.seriesId, studyId: c.data?.study.ID ?? null, kind: c.kind, orientation: c.orientation })),
      tool: this.tool,
      linked: this.linked,
      overlays: this.overlaysOn,
      preset: cell?.preset ?? 0,
      invert: cell?.invert ?? false,
      playing: cell?.playing ?? false,
      index: cell?.index ?? 0,
      count: cell?.count ?? 0,
      instanceFile: slot ? `instances/${slot.instance.ID}/file` : null,
      meta: slot?.meta ?? null,
      canMpr: this.planesPossible(cell?.data ?? null),
      canAdd: this.cells.length < 6,
      mprBlocked: getShouldUseCPURendering()
        ? "MPR needs WebGL, which this browser doesn't offer"
        : cell?.data && cell.data.slots.length < 8
          ? "MPR needs a series of at least 8 slices"
          : null,
      loadingVolume: this.loadingVolume,
    });
  }

  // ---- series and cells ----

  private loadSeries(id: string) {
    if (!this.loaded.has(id)) {
      this.loaded.set(
        id,
        (async () => {
          const { study, series } = await this.resolve(id);
          this.loadAnnotations(study.MainDicomTags.StudyInstanceUID);
          const base = new URL(`studies/${study.MainDicomTags.StudyInstanceUID}/series/${series.MainDicomTags.SeriesInstanceUID}/`, WEB);
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
          return { study, series, slots };
        })().catch((e) => {
          this.loaded.delete(id);
          throw e;
        }),
      );
    }
    return this.loaded.get(id)!;
  }

  private cellIds = 0;

  private groupOf(cell: Pick<Cell, "group">) {
    return cell.group === "stack" ? this.stackGroup : cell.group === "mpr" ? this.mprGroup : this.vrGroup;
  }

  private makeCell(i: number, kind: Cell["kind"], orientation: Orientation | null, group: Cell["group"] = kind === "mpr" ? "mpr" : kind, before?: HTMLElement): Cell {
    const div = (cls: string) => Object.assign(document.createElement("div"), { className: cls });
    const element = div("cs absolute inset-0") as HTMLDivElement;
    const ov = Object.fromEntries((["tl", "tr", "bl", "br", "ol", "or", "ot", "ob"] as const).map((k) => [k, div(`ov ov-${k}`)])) as Cell["ov"];
    const note = div("ov-note");
    const scale = div("ov ov-scale");
    // the position in the series, a line along the top
    const bar = div("cell-bar");
    const dotEl = div("cell-bar-dot");
    bar.append(dotEl);
    // a small bar of menus at the bottom: window preset, and the slab of an MPR view
    const menus = div("cell-menus");
    const select = (options: string[], label: string) => {
      const s = document.createElement("select");
      s.setAttribute("aria-label", label);
      s.append(...options.map((o, j) => Object.assign(document.createElement("option"), { value: String(j), textContent: o })));
      return s;
    };
    // in a 3D view, how the volume is rendered (the CT list until the series is known)
    const presetSelect = kind === "3d" ? select(RENDERS.CT.map((r) => r.name), "Rendering") : select(PRESETS.map((p) => p.name), "Window preset");
    const slabSelect = select(SLABS.map((s) => s.name), "Slab");
    // the plane any viewport outside MPR shows: the images as acquired, or a plane of the volume
    const planeSelect = select(PLANES.map((p) => p.name), "Plane");
    planeSelect.value = String(PLANES.findIndex((p) => p.plane === (kind === "3d" ? "3d" : (orientation ?? "original"))));
    const remove = Object.assign(document.createElement("button"), { type: "button", className: "cell-close", textContent: "×", title: "Remove this view" });
    remove.setAttribute("aria-label", "Remove this view");
    if (group !== "mpr") menus.append(planeSelect);
    menus.append(presetSelect);
    if (kind === "mpr") menus.append(slabSelect);
    const wrap = div("cell") as HTMLDivElement;
    wrap.tabIndex = 0;
    if (orientation) wrap.dataset.plane = orientation;
    if (kind === "3d") wrap.dataset.kind = "3d";
    wrap.append(element, bar, ...Object.values(ov), scale, note, menus);
    if (group !== "mpr") menus.append(remove);
    this.grid.insertBefore(wrap, before ?? null);
    const id = `vp${++this.cellIds}-${kind}${orientation ? `-${orientation}` : ""}`;
    this.engine.enableElement({
      viewportId: id,
      type: kind === "stack" ? Enums.ViewportType.STACK : kind === "3d" ? Enums.ViewportType.VOLUME_3D : Enums.ViewportType.ORTHOGRAPHIC,
      element,
      defaultOptions: {
        background: [0, 0, 0],
        // a 3D view starts from the front, as the patient faces you
        ...(kind === "3d" ? { orientation: Enums.OrientationAxis.CORONAL } : orientation ? { orientation: orientation as Enums.OrientationAxis } : {}),
      },
    });
    this.groupOf({ group }).addViewport(id, ENGINE);
    const cell: Cell = {
      i, id, kind, orientation, group, element, wrap, ov, note, bar, dot: dotEl, scale, presetSelect, slabSelect, planeSelect,
      viewport: this.engine.getViewport(id),
      seriesId: null, data: null, index: 0, count: 0, preset: 0, slab: 0, invert: false, rotation: 0, fitZoom: null, failed: new Set(), playing: false,
    };
    presetSelect.addEventListener("change", () => this.setPreset(Number(presetSelect.value), cell));
    slabSelect.addEventListener("change", () => this.setSlab(Number(slabSelect.value), cell));
    planeSelect.addEventListener("change", () => this.setPlane(PLANES[Number(planeSelect.value)].plane, cell));
    remove.addEventListener("click", () => this.removeView(cell));
    for (const s of [presetSelect, slabSelect, planeSelect, remove]) s.addEventListener("pointerdown", (e) => e.stopPropagation());
    wrap.addEventListener("pointerdown", () => this.activate(cell.i), true);
    this.lens(cell);
    wrap.addEventListener("contextmenu", (e) => e.preventDefault());
    wrap.addEventListener("dblclick", (e) => !(e.target as HTMLElement).closest("select, button") && this.reset(cell));
    // a series dropped from the study browser
    wrap.addEventListener("dragover", (e) => {
      if (e.dataTransfer?.types.includes("application/x-pacs-series")) {
        e.preventDefault();
        wrap.classList.add("drop");
      }
    });
    wrap.addEventListener("dragleave", () => wrap.classList.remove("drop"));
    wrap.addEventListener("drop", (e) => {
      const id = e.dataTransfer?.getData("application/x-pacs-series");
      wrap.classList.remove("drop");
      if (!id) return;
      e.preventDefault();
      e.stopPropagation();
      this.activate(cell.i);
      if (this.mode === "mpr") this.exitMpr().then(() => this.showSeries(id));
      else this.show(cell, id);
    });
    element.addEventListener(Enums.Events.STACK_NEW_IMAGE, ((e: any) => {
      cell.index = e.detail.imageIdIndex;
      this.overlays(cell);
      if (this.linked && !this.syncing && cell === this.active()) this.syncFrom(cell);
      if (cell === this.active()) this.emit();
    }) as EventListener);
    element.addEventListener(Enums.Events.VOLUME_NEW_IMAGE, (() => {
      this.overlays(cell);
      if (cell === this.active()) this.emit();
    }) as EventListener);
    element.addEventListener(Enums.Events.VOI_MODIFIED, () => {
      const w = PRESETS[cell.preset]?.window;
      if (w) {
        const voi = (cell.viewport as any).getProperties()?.voiRange;
        const want = utilities.windowLevel.toLowHighRange(w.width, w.center);
        if (voi && (Math.abs(voi.lower - want.lower) > 0.5 || Math.abs(voi.upper - want.upper) > 0.5)) {
          cell.preset = -1;
          cell.presetSelect.value = "";
          if (cell === this.active()) this.emit();
        }
      }
      this.overlays(cell);
    });
    element.addEventListener(Enums.Events.CAMERA_MODIFIED, () => this.overlays(cell));
    return cell;
  }

  /** Called with a message when WebGL drew nothing; the page then reloads drawing on the CPU. */
  onBlank: ((message: string) => void) | null = null;
  private checkedDrawing = false;

  // once per page, after the first image: if WebGL left the viewport's canvas empty while the
  // image has content, this browser can't show WebGL output, so draw on the CPU from now on
  private checkDrawn(cell: Cell) {
    if (this.checkedDrawing || getShouldUseCPURendering()) return;
    this.checkedDrawing = true;
    setTimeout(() => {
      const canvas = cell.element.querySelector("canvas");
      const image = (cell.viewport as any).getImageData?.();
      if (!canvas || !image) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const { width, height } = canvas;
      const pixels = ctx.getImageData(0, 0, width, height).data;
      let lit = 0;
      for (let i = 0; i < pixels.length; i += 4 * 97) if (pixels[i] | pixels[i + 1] | pixels[i + 2]) lit++;
      const range = image.scalarData ? image.imageData?.getPointData?.().getScalars?.().getRange?.() : null;
      const flat = range && range[0] === range[1];
      if (lit === 0 && !flat) {
        // one step down: the read back keeps WebGL (and MPR); only if that is blank too, the CPU
        if (renderingMode() === "gpu") {
          setRenderingMode("readback");
          this.onBlank?.("This browser didn't show the WebGL image; the viewer now reads it out of WebGL itself.");
        } else {
          setRenderingMode("cpu");
          this.onBlank?.("This browser's WebGL draws blank images, so the viewer now draws on the CPU. 3 planes and other planes need WebGL and are off.");
        }
      }
    }, 600);
  }

  // the magnifying glass: while the left button is held, a round lens over the pointer shows
  // what is under it larger, from the image as drawn (any viewport, any plane); the wheel
  // changes how much, 1.5 to 8 times
  private lensZoom = 3;
  private lens(cell: Cell) {
    const SIZE = 240;
    let lens: HTMLCanvasElement | null = null;
    const draw = (e: PointerEvent) => {
      const source = cell.element.querySelector("canvas");
      if (!lens || !source) return;
      const box = cell.wrap.getBoundingClientRect();
      const x = e.clientX - box.left, y = e.clientY - box.top;
      lens.style.left = `${x - SIZE / 2}px`;
      lens.style.top = `${y - SIZE / 2}px`;
      const dpr = source.width / Math.max(source.clientWidth, 1);
      const ctx = lens.getContext("2d")!;
      const out = lens.width;
      const span = (SIZE / this.lensZoom) * dpr;
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, out, out);
      ctx.imageSmoothingEnabled = this.lensZoom < 4;
      ctx.drawImage(source, x * dpr - span / 2, y * dpr - span / 2, span, span, 0, 0, out, out);
      lens.dataset.zoom = `${this.lensZoom.toFixed(1).replace(/\.0$/, "")}×`;
    };
    cell.wrap.addEventListener("pointerdown", (e) => {
      if (this.tool !== "magnify" || e.button !== 0 || e.shiftKey || (e.target as HTMLElement).closest("select, button")) return;
      lens = document.createElement("canvas");
      lens.className = "lens";
      lens.width = lens.height = Math.round(SIZE * (devicePixelRatio || 1));
      lens.style.width = lens.style.height = `${SIZE}px`;
      cell.wrap.append(lens);
      cell.wrap.setPointerCapture(e.pointerId);
      draw(e);
    });
    cell.wrap.addEventListener("pointermove", (e) => lens && draw(e));
    const end = () => {
      lens?.remove();
      lens = null;
    };
    cell.wrap.addEventListener("pointerup", end);
    cell.wrap.addEventListener("pointercancel", end);
    let last: PointerEvent | null = null;
    cell.wrap.addEventListener("pointermove", (e) => (last = e));
    cell.wrap.addEventListener(
      "wheel",
      (e) => {
        if (!lens) return;
        // while the lens is out, the wheel is its magnification, not the next image
        e.preventDefault();
        e.stopPropagation();
        this.lensZoom = Math.max(1.5, Math.min(8, this.lensZoom * (e.deltaY < 0 ? 1.25 : 0.8)));
        if (last) draw(last);
      },
      { capture: true, passive: false },
    );
  }

  private dropCell(cell: Cell, remove = true) {
    if (cell.playing) this.cine(false, cell);
    try {
      toolUtilities.stackPrefetch.disable(cell.element);
    } catch {}
    this.groupOf(cell).removeViewports(ENGINE, cell.id);
    if (remove) {
      this.engine.disableElement(cell.id);
      cell.wrap.remove();
    }
  }

  private async show(cell: Cell, id: string, at: number | null = null) {
    if (cell.playing) this.cine(false, cell);
    cell.seriesId = id;
    cell.note.textContent = "Loading…";
    this.emit();
    let data: Loaded;
    try {
      data = await this.loadSeries(id);
    } catch {
      cell.note.textContent = "This series could not be loaded";
      return;
    }
    if (cell.seriesId !== id) return;
    Object.assign(cell, { data, failed: new Set(), preset: 0, rotation: 0, count: data.slots.length });
    cell.presetSelect.value = "0";
    if (!data.slots.length) {
      cell.note.textContent = "This series has no images";
      return this.emit();
    }
    cell.index = at ?? Math.floor(data.slots.length / 2);
    const viewport = cell.viewport as Types.IStackViewport;
    try {
      await viewport.setStack(
        data.slots.map((s) => s.imageId),
        cell.index,
      );
      if (cell.invert) viewport.setProperties({ invert: true });
    } catch {
      cell.failed.add(data.slots[cell.index].imageId);
    }
    viewport.render();
    this.measureFit(cell);
    this.checkDrawn(cell);
    toolUtilities.stackPrefetch.enable(cell.element);
    this.overlays(cell);
    this.emit();
  }

  // the cm scale on the image: the tool can only start once an image is drawn
  private measureFit(cell: Cell) {
    try {
      cell.fitZoom = (cell.viewport as any).getZoom();
    } catch {}
  }

  private overlays(cell: Cell) {
    const { ov, data } = cell;
    if (!data?.slots.length) return;
    const p = data.study.PatientMainDicomTags, t = data.study.MainDicomTags, st = data.series.MainDicomTags;
    let slot: Slot, index: number, count: number;
    if (cell.kind === "3d") {
      slot = data.slots[Math.floor(data.slots.length / 2)];
      index = 0;
      count = 1;
    } else if (cell.kind === "stack") {
      slot = data.slots[cell.index];
      index = cell.index;
      count = data.slots.length;
    } else {
      const vp = cell.viewport as Types.IVolumeViewport;
      index = vp.getSliceIndex?.() ?? 0;
      count = vp.getNumberOfSlices?.() ?? 1;
      slot = data.slots[Math.floor(data.slots.length / 2)];
    }
    cell.index = index;
    cell.count = count;
    const pos = cell.kind === "stack" ? tag(slot.meta, "00200032") : null;
    const thickness = num(slot.meta, "00180050", NaN);
    const columns = num(slot.meta, "00280011", 0), rows = num(slot.meta, "00280010", 0);
    const voi = (cell.viewport as any).getProperties?.()?.voiRange;
    let zoom = 1;
    try {
      if (cell.fitZoom) zoom = (cell.viewport as any).getZoom() / cell.fitZoom;
    } catch {}
    const wl = voi ? utilities.windowLevel.toWindowLevel(voi.lower, voi.upper) : null;
    ov.tl.textContent = [
      `${st.Modality || ""} ${st.SeriesNumber ?? ""}  ${st.SeriesDescription || ""}`.trim(),
      cell.kind === "3d" ? `3D  ·  ${rendersOf(data)[cell.preset]?.name ?? ""}` : cell.orientation ? `${cell.orientation[0].toUpperCase()}${cell.orientation.slice(1)}${SLABS[cell.slab].mm ? `  ·  ${SLABS[cell.slab].name}` : ""}` : rows ? `${columns} × ${rows}` : "",
    ].filter(Boolean).join("\n");
    ov.tr.textContent = `${personName(p.PatientName)}\n${[p.PatientID, sexAge(p.PatientSex, p.PatientBirthDate, t.StudyDate)].filter(Boolean).join("  ·  ")}\n${t.StudyDescription || ""}\n${formatDate(t.StudyDate)} ${formatTime(t.StudyTime)}`;
    ov.bl.textContent = cell.kind === "3d" ? `Zoom ${Math.round(zoom * 100)}%` : [
      `Image ${index + 1} / ${count}${slot.frames > 1 ? `  ·  frame ${slot.frame}` : ""}`,
      wl ? `W ${Math.round(wl.windowWidth)}  L ${Math.round(wl.windowCenter)}` : "",
      `Zoom ${Math.round(zoom * 100)}%`,
    ].filter(Boolean).join("\n");
    ov.br.textContent = [pos ? `Z ${Number(pos[2]).toFixed(1)} mm` : null, isFinite(thickness) && cell.kind === "stack" ? `Thickness ${thickness} mm` : null].filter(Boolean).join("\n");
    try {
      for (const [k, v] of Object.entries(markers(cell.viewport))) ov[k as keyof Cell["ov"]].textContent = v;
    } catch {}
    cell.dot.style.left = count > 1 ? `${(100 * index) / (count - 1)}%` : "50%";
    // a rendering has no one scale: what is near and far differ
    if (cell.kind === "3d") cell.scale.innerHTML = "";
    else this.drawScale(cell);
    cell.note.textContent = cell.kind === "stack" && cell.failed.has(slot.imageId) ? "This image could not be decoded" : "";
  }

  // a ruler along the right edge, as on film: the longest round length that fits in 40% of
  // the viewport's height, a tick per centimetre
  private drawScale(cell: Cell) {
    try {
      const vp = cell.viewport;
      const { focalPoint, viewUp } = vp.getCamera();
      const a = vp.worldToCanvas(focalPoint!);
      const b = vp.worldToCanvas(focalPoint!.map((x, i) => x + viewUp![i] * 10) as Types.Point3);
      const perCm = Math.hypot(b[0] - a[0], b[1] - a[1]);
      const room = cell.wrap.clientHeight * 0.4;
      const cm = [50, 20, 10, 5, 2, 1].find((n) => n * perCm <= room);
      if (!cm || !isFinite(perCm) || perCm <= 0) return void (cell.scale.innerHTML = "");
      const h = Math.round(cm * perCm);
      const every = perCm >= 6 ? 1 : 5;
      const ticks = Array.from({ length: Math.floor(cm / every) + 1 }, (_, i) => {
        const y = Math.round(i * every * perCm) + 0.5;
        const long = i === 0 || i * every === cm || (i * every) % 5 === 0;
        return `<line x1="${long ? 0 : 4}" x2="10" y1="${y}" y2="${y}"/>`;
      }).join("");
      cell.scale.innerHTML = `<svg width="11" height="${h + 1}" viewBox="0 0 11 ${h + 1}"><line x1="10" x2="10" y1="0" y2="${h + 1}"/>${ticks}</svg><span>${cm} cm</span>`;
    } catch {
      cell.scale.innerHTML = "";
    }
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
      this.measureFit(cell);
      this.overlays(cell);
    }
  }

  // linked scrolling: the slice of each other viewport nearest to this one's, along its normal
  private syncFrom(source: Cell) {
    if (source.kind !== "stack" || !source.data?.slots.length) return;
    const slot = source.data.slots[source.index];
    const frameOfReference = first(slot.meta, "00200052", null);
    const pos = tag(slot.meta, "00200032");
    if (!pos) return;
    this.syncing = true;
    for (const cell of this.cells) {
      if (cell === source || cell.kind !== "stack" || !cell.data?.slots.length) continue;
      if (frameOfReference && first(cell.data.slots[0].meta, "00200052", null) !== frameOfReference) continue;
      const iop = (tag(cell.data.slots[0].meta, "00200037") || [1, 0, 0, 0, 1, 0]).map(Number);
      const normal = cross(iop.slice(0, 3), iop.slice(3, 6));
      const target = dot(pos.map(Number), normal);
      let best = cell.index, bestDistance = Infinity;
      cell.data.slots.forEach((s, j) => {
        const q = tag(s.meta, "00200032");
        if (!q) return;
        const d = Math.abs(dot(q.map(Number), normal) - target);
        if (d < bestDistance) [best, bestDistance] = [j, d];
      });
      if (best !== cell.index) (cell.viewport as Types.IStackViewport).setImageIdIndex(best);
    }
    this.syncing = false;
  }

  // the left button runs the chosen tool; pan stays on the middle button and shift-drag,
  // zoom on the right button, whatever is chosen
  private bindings(name: ToolName, left: boolean) {
    const { MouseBindings, KeyboardBindings } = ToolEnums;
    const b: any[] = [];
    // the magnifying glass is ours (see `lens`), not Cornerstone's: no binding for it
    if (name === "magnify") return b;
    if (left) b.push({ mouseButton: MouseBindings.Primary });
    if (name === "pan") b.push({ mouseButton: MouseBindings.Auxiliary }, { mouseButton: MouseBindings.Primary, modifierKey: KeyboardBindings.Shift });
    if (name === "zoom") b.push({ mouseButton: MouseBindings.Secondary });
    return b;
  }
  private applyTool(from: ToolName | null, to: ToolName) {
    for (const group of [this.stackGroup, this.mprGroup]) {
      for (const name of Object.keys(TOOLS) as ToolName[]) {
        // crosshairs need the three MPR views in place: enterMpr turns them on
        if (name === "crosshairs" && (group === this.stackGroup || this.mode !== "mpr")) continue;
        if (from && name !== from && name !== to && name !== "pan" && name !== "zoom") continue;
        const b = this.bindings(name, name === to);
        if (b.length) group.setToolActive(TOOLS[name].toolName, { bindings: b });
        else if (name === "crosshairs") group.setToolEnabled(TOOLS[name].toolName);
        else group.setToolPassive(TOOLS[name].toolName);
      }
    }
  }

  // ---- what the toolbar calls ----

  useTool(name: ToolName) {
    if (name === this.tool) return;
    if (name === "crosshairs" && this.mode !== "mpr") return;
    this.applyTool(this.tool, name);
    this.tool = name;
    this.grid.dataset.tool = name;
    this.emit();
  }

  async setLayout(i: number, fill = true) {
    if (this.mode === "mpr") await this.exitMpr();
    const { cols, rows } = LAYOUTS[i];
    this.layout = i;
    this.setGrid(cols, rows);
    const n = cols * rows;
    while (this.cells.length > n) this.dropCell(this.cells.pop()!);
    const fresh: Cell[] = [];
    while (this.cells.length < n) {
      const cell = this.makeCell(this.cells.length, "stack", null);
      this.cells.push(cell);
      fresh.push(cell);
    }
    this.refit();
    if (this.current >= this.cells.length) this.current = 0;
    this.frame();
    if (!fill) return this.emit();
    // new viewports show the series after the first one's, in its study
    const firstData = this.cells[0].data;
    const siblings: string[] = firstData ? (await this.seriesOfStudy(firstData.study.ID)) : [];
    const start = Math.max(0, siblings.indexOf(this.cells[0].seriesId ?? ""));
    await Promise.all(fresh.map((cell) => (siblings.length ? this.show(cell, siblings[(start + cell.i) % siblings.length]) : null)));
    if (this.linked) this.syncFrom(this.active());
    this.emit();
  }

  private seriesOfStudyCache = new Map<string, Promise<string[]>>();
  private seriesOfStudy(studyId: string) {
    if (!this.seriesOfStudyCache.has(studyId))
      this.seriesOfStudyCache.set(
        studyId,
        api<any[]>(`studies/${studyId}/series`).then((list) =>
          list.sort((a, b) => Number(a.MainDicomTags.SeriesNumber ?? 1e9) - Number(b.MainDicomTags.SeriesNumber ?? 1e9)).map((s) => s.ID),
        ),
      );
    return this.seriesOfStudyCache.get(studyId)!;
  }

  /** A hanging protocol's plan: this layout, these series in these viewports. */
  async hang(layout: number, series: (string | null)[], linked: boolean, mpr = false) {
    if (this.mode === "mpr") await this.exitMpr();
    // viewports keep nothing of the previous arrangement
    for (const cell of this.cells) this.dropCell(cell);
    this.cells = [];
    this.current = 0;
    await this.setLayout(layout, false);
    await Promise.all(this.cells.map((cell, i) => (series[i] ? this.show(cell, series[i]!) : null)));
    for (const cell of this.cells) if (!series[cell.i]) cell.note.textContent = "Drop a series here";
    this.setLinked(linked);
    if (mpr) await this.enterMpr();
    this.cells[0]?.wrap.focus();
  }

  async showSeries(id: string) {
    if (this.mode === "mpr") await this.exitMpr();
    return this.show(this.active(), id);
  }
  async nextSeries(step: number) {
    const cell = this.active();
    if (!cell.data || this.mode === "mpr") return;
    const list = await this.seriesOfStudy(cell.data.study.ID);
    const next = list[list.indexOf(cell.seriesId!) + step];
    if (next) return this.show(cell, next);
  }

  // ---- MPR ----

  /** The series as a volume, made and loading once. */
  private async volumeOf(data: Loaded) {
    const volumeId = `cornerstoneStreamingImageVolume:${data.series.ID}`;
    if (!cache.getVolume(volumeId)) {
      const volume = await volumeLoader.createAndCacheVolume(volumeId, { imageIds: data.slots.map((s) => s.imageId) });
      (volume as any).load?.();
    }
    return volumeId;
  }

  /** Whether a series can be shown in other planes: enough slices, WebGL. */
  private planesPossible(data: Loaded | null) {
    return !getShouldUseCPURendering() && !!data && data.slots.length >= 8 && data.slots.every((s) => s.frames === 1);
  }

  // ---- viewports: their plane, adding and removing them ----

  /** One viewport's plane: its series as acquired, or a plane of its volume. */
  async setPlane(plane: Plane, cell = this.active()) {
    if (this.mode === "mpr" || !cell.data) return;
    if (plane !== "original" && !this.planesPossible(cell.data)) {
      cell.planeSelect.value = String(PLANES.findIndex((p) => p.plane === planeOf(cell)));
      cell.note.textContent = getShouldUseCPURendering() ? "Other planes need WebGL" : "Other planes need a series of at least 8 slices";
      setTimeout(() => (cell.note.textContent = ""), 2500);
      return;
    }
    const data = cell.data;
    const next =
      plane === "original"
        ? this.makeCell(cell.i, "stack", null, "stack", cell.wrap)
        : plane === "3d"
          ? this.makeCell(cell.i, "3d", null, "3d", cell.wrap)
          : this.makeCell(cell.i, "mpr", plane, "stack", cell.wrap);
    next.invert = cell.invert;
    this.dropCell(cell);
    this.cells[cell.i] = next;
    this.frame();
    this.refit();
    if (plane === "original") await this.show(next, data.series.ID);
    else {
      Object.assign(next, { seriesId: data.series.ID, data, count: 1 });
      next.note.textContent = "Loading the volume…";
      this.emit();
      try {
        const volumeId = await this.volumeOf(data);
        await (next.viewport as Types.IVolumeViewport).setVolumes([{ volumeId }]);
        next.note.textContent = "";
        if (plane === "3d") {
          const renders = rendersOf(data);
          next.presetSelect.replaceChildren(...renders.map((r, j) => Object.assign(document.createElement("option"), { value: String(j), textContent: r.name })));
          this.setPreset(0, next);
        } else if (next.invert) (next.viewport as any).setProperties({ invert: true });
        next.viewport.render();
        this.measureFit(next);
      } catch {
        next.note.textContent = "This series can't be shown in that plane";
      }
    }
    this.overlays(next);
    this.emit();
  }

  private layoutFor(n: number) {
    return n <= 1 ? 0 : n === 2 ? 1 : n === 3 ? 4 : n === 4 ? 3 : 5;
  }

  /** One more viewport: the active series in `plane`, or the study's next series not yet
   * shown. Without a choice, the next plane of the active series not yet on screen (sagittal,
   * coronal, axial) if it has the slices for it, else the next series. */
  async addView(choice: { plane?: Exclude<Plane, "original">; nextSeries?: boolean } = {}) {
    if (this.mode === "mpr") await this.exitMpr();
    if (this.cells.length >= 6) return;
    const active = this.active();
    const keep = this.cells.map((c) => ({ seriesId: c.seriesId, plane: planeOf(c) }));
    let seriesId = active.seriesId;
    let plane: Exclude<Plane, "original"> | null = choice.plane ?? null;
    if (!plane && !choice.nextSeries && this.planesPossible(active.data)) {
      const onScreen = this.cells.filter((c) => c.seriesId === active.seriesId).map((c) => c.orientation);
      plane = (["sagittal", "coronal", "axial"] as const).find((p) => !onScreen.includes(p)) ?? null;
    }
    if (!plane) {
      const shown = this.cells.map((c) => c.seriesId);
      const studyId = active.data?.study.ID;
      const list = studyId ? await this.seriesOfStudy(studyId) : [];
      const after = list.indexOf(active.seriesId ?? "");
      seriesId = [...list.slice(after + 1), ...list.slice(0, after + 1)].find((id) => !shown.includes(id)) ?? active.seriesId;
    }
    await this.hang(this.layoutFor(keep.length + 1), [...keep.map((k) => k.seriesId), seriesId], this.linked);
    // the planes the viewports had stay as they were
    for (const [i, k] of keep.entries()) if (k.plane !== "original") await this.setPlane(k.plane, this.cells[i]);
    const added = this.cells[this.cells.length - 1];
    if (plane && added) await this.setPlane(plane, added);
    this.activate(this.cells.length - 1);
  }

  /** Remove a viewport; the others close up. */
  async removeView(cell: Cell) {
    if (this.cells.length <= 1) return;
    const keep = this.cells.filter((c) => c !== cell).map((c) => ({ seriesId: c.seriesId, plane: planeOf(c) }));
    await this.hang(this.layoutFor(keep.length), keep.map((k) => k.seriesId), this.linked);
    for (const [i, k] of keep.entries()) if (k.plane !== "original") await this.setPlane(k.plane, this.cells[i]);
  }

  /** Into MPR; if the volume can't be built, back to the stacks as they were. */
  async enterMpr(): Promise<string | null> {
    try {
      await this.buildMpr();
      return null;
    } catch (e: any) {
      this.loadingVolume = false;
      await this.exitMpr();
      return e?.message || "MPR failed";
    }
  }

  private async buildMpr() {
    const source = this.active();
    if (this.mode === "mpr" || !source.data || source.data.slots.length < 8 || getShouldUseCPURendering()) return;
    const data = source.data;
    this.stackLayout = this.layout;
    this.stackSeries = this.cells.map((c) => c.seriesId ?? "");
    this.loadingVolume = true;
    this.emit();
    for (const cell of this.cells) this.dropCell(cell);
    this.cells = [];
    this.mode = "mpr";
    this.setGrid(2, 2);
    const planes: Orientation[] = ["axial", "sagittal", "coronal", "coronal"];
    this.cells = planes.map((o, i) => this.makeCell(i, "mpr", o));
    this.current = 0;
    this.frame();
    this.refit();
    const volumeId = await this.volumeOf(data);
    for (const cell of this.cells) Object.assign(cell, { seriesId: data.series.ID, data, count: 1 });
    await setVolumesForViewports(this.engine, [{ volumeId }], this.cells.map((c) => c.id));
    // the fourth view: the whole volume as a coronal MIP, like a radiograph
    this.setSlab(SLABS.length - 1, this.cells[3]);
    // crosshairs first, on the left button: moving them is what MPR is for
    this.applyTool(this.tool, "crosshairs");
    this.tool = "crosshairs";
    this.grid.dataset.tool = "crosshairs";
    this.loadingVolume = false;
    this.engine.render();
    this.refit();
    this.emit();
  }

  async exitMpr() {
    if (this.mode !== "mpr") return;
    this.mprGroup.setToolDisabled(CrosshairsTool.toolName);
    for (const cell of this.cells) this.dropCell(cell);
    this.cells = [];
    this.mode = "stack";
    if (this.tool === "crosshairs") {
      this.tool = "windowLevel";
      this.applyTool(null, "windowLevel");
    }
    const { cols, rows } = LAYOUTS[this.stackLayout];
    this.layout = this.stackLayout;
    this.setGrid(cols, rows);
    this.cells = Array.from({ length: cols * rows }, (_, i) => this.makeCell(i, "stack", null));
    this.current = 0;
    this.frame();
    this.refit();
    await Promise.all(this.cells.map((cell, i) => (this.stackSeries[i] ? this.show(cell, this.stackSeries[i]) : null)));
    this.emit();
  }

  setSlab(i: number, cell = this.active()) {
    if (cell.kind !== "mpr") return;
    const vp = cell.viewport as Types.IVolumeViewport;
    cell.slab = i;
    cell.slabSelect.value = String(i);
    const mm = SLABS[i].mm;
    vp.setBlendMode(mm ? Enums.BlendModes.MAXIMUM_INTENSITY_BLEND : Enums.BlendModes.COMPOSITE);
    if (mm) vp.setSlabThickness(mm);
    else vp.resetSlabThickness();
    vp.render();
    this.overlays(cell);
  }

  // ---- image ----

  go(i: number) {
    const cell = this.active();
    if (cell.kind !== "stack" || !cell.data) return;
    const n = Math.max(0, Math.min(cell.data.slots.length - 1, i));
    const vp = cell.viewport as Types.IStackViewport;
    if (n !== vp.getCurrentImageIdIndex()) vp.setImageIdIndex(n);
  }
  step(n: number) {
    const cell = this.active();
    if (cell.kind === "3d") return;
    if (cell.kind === "stack") this.go(cell.index + n);
    else utilities.scroll(cell.viewport as any, { delta: n });
  }
  first() {
    this.go(0);
  }
  last() {
    this.go(this.active().count - 1);
  }

  setPreset(i: number, cell = this.active()) {
    if (cell.kind === "3d") {
      const render = rendersOf(cell.data)[i];
      if (!render) return;
      cell.preset = i;
      cell.presetSelect.value = String(i);
      (cell.viewport as any).setProperties({ preset: render.preset });
      cell.viewport.render();
      this.overlays(cell);
      return this.emit();
    }
    cell.preset = i;
    cell.presetSelect.value = String(i);
    const w = PRESETS[i].window;
    const vp = cell.viewport as any;
    if (w) vp.setProperties({ voiRange: utilities.windowLevel.toLowHighRange(w.width, w.center) });
    else {
      vp.resetProperties();
      vp.setProperties({ invert: cell.invert });
    }
    vp.render();
    this.emit();
  }
  toggleInvert(cell = this.active()) {
    if (cell.kind === "3d") return;
    cell.invert = !cell.invert;
    (cell.viewport as any).setProperties({ invert: cell.invert });
    cell.viewport.render();
    this.emit();
  }
  /** Turn the image by `degrees`: 90 to the right, -90 to the left. */
  rotate(degrees = 90, cell = this.active()) {
    cell.rotation = (((cell.rotation + degrees) % 360) + 360) % 360;
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
    this.measureFit(cell);
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
  /** Remove every measurement (saved as removed); what was removed, for an undo. */
  clearMeasurements() {
    const removed = [...annotation.state.getAllAnnotations()];
    for (const a of removed) this.studyOf(a);
    annotation.state.removeAllAnnotations();
    this.cells.forEach((c) => c.viewport.render());
    return removed;
  }
  /** Put measurements back, as `clearMeasurements` returned them. */
  restoreMeasurements(list: any[]) {
    const studies = new Set<string>();
    for (const a of list) {
      if (annotation.state.getAnnotation(a.annotationUID)) continue;
      a.invalidated = true;
      annotation.state.addAnnotation(a, undefined as any);
      const study = this.studyOf(a);
      if (study) studies.add(study);
    }
    for (const study of studies) this.saveSoon(study);
    this.cells.forEach((c) => c.viewport.render());
  }
  cine(on?: boolean, cell = this.active()) {
    on ??= !cell.playing;
    if (cell.count < 2 || cell.kind === "3d") return;
    cell.playing = on;
    if (on) toolUtilities.cine.playClip(cell.element, { framesPerSecond: 15, loop: true });
    else toolUtilities.cine.stopClip(cell.element);
    this.emit();
  }
  /** The active viewport as drawn (image, measurements and text), as a PNG. */
  exportImage() {
    const cell = this.active();
    const canvas = cell.element.querySelector("canvas");
    if (!canvas) return;
    const out = document.createElement("canvas");
    out.width = canvas.width;
    out.height = canvas.height;
    const ctx = out.getContext("2d")!;
    ctx.drawImage(canvas, 0, 0);
    // the measurements are an SVG layer over the canvas
    const svg = cell.element.querySelector("svg");
    const finish = () => {
      const a = document.createElement("a");
      const st = cell.data?.series.MainDicomTags ?? {};
      a.download = `${st.Modality || "image"}-${st.SeriesNumber ?? ""}-${cell.index + 1}.png`;
      a.href = out.toDataURL("image/png");
      a.click();
    };
    if (!svg) return finish();
    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0, out.width, out.height);
      finish();
    };
    img.onerror = finish;
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(svg))}`;
  }
}
