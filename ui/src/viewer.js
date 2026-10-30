// The image viewer: one to four Cornerstone3D stack viewports side by side. Frames come over
// WADO-RS in the transfer syntax they are stored in (JPEG-LS by default) and are decoded in web
// workers by dicom-image-loader's codecs; each series' metadata is fetched once and handed to
// the loader, so no image needs a request of its own for it.
//
// The toolbar acts on the active viewport (the framed one); the rail loads a series into it.
// With Link on, scrolling one viewport moves the others of the same frame of reference to the
// nearest slice.

import { init as coreInit, RenderingEngine, Enums, eventTarget, utilities } from "@cornerstonejs/core";
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
import { h, searchIcon, HttpError, api, personName, dicomDate, dicomTime, sexAge, plural, setStatus, isTyping, ROOT, WEB } from "./util.js";
import { sortSeries, seriesThumb } from "./thumbs.js";
import { first, tag, num } from "./pixels.js";
import { tagRows } from "./tags.js";

// what the left mouse button does: [tool, label, key, title]
const LEFT_TOOLS = [
  [WindowLevelTool, "W/L", "w", "Window/level"],
  [LengthTool, "Len", "l", "Length"],
  [ProbeTool, "HU", "h", "Pixel value"],
  [AngleTool, "Ang", "a", "Angle"],
  [CobbAngleTool, "Cobb", "c", "Cobb angle"],
  [BidirectionalTool, "Bi", "b", "Bidirectional: long and short axis"],
  [EllipticalROITool, "ROI", "r", "Elliptical ROI: area, mean, std"],
  [RectangleROITool, "Rect", "g", "Rectangle ROI: area, mean, std"],
];

const PRESETS = [
  ["Auto", null],
  ["Soft", { center: 40, width: 400 }],
  ["Liver", { center: 60, width: 160 }],
  ["Lung", { center: -600, width: 1500 }],
  ["Bone", { center: 400, width: 1800 }],
  ["Brain", { center: 40, width: 80 }],
];

// [label, columns, rows]
const LAYOUTS = [
  ["1×1", 1, 1],
  ["1×2", 2, 1],
  ["2×1", 1, 2],
  ["2×2", 2, 2],
];

const ENGINE = "pacs-rs";
const TOOLS = "pacs-rs-tools";

let ready = null;
function initCornerstone() {
  ready ??= (async () => {
    await coreInit();
    loaderInit({ maxWebWorkers: Math.max(1, Math.min(4, navigator.hardwareConcurrency || 1)) });
    toolsInit();
    // measurements in the page's amber and monospace; white while being moved
    const amber = "rgb(242, 169, 59)", white = "rgb(241, 242, 243)";
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
        textBoxFontFamily: 'ui-monospace, "JetBrains Mono", "DejaVu Sans Mono", Menlo, monospace',
        textBoxFontSize: "12px",
        textBoxBackground: "rgba(0, 0, 0, 0.6)",
      },
    });
    for (const tool of [PanTool, ZoomTool, StackScrollTool, ...LEFT_TOOLS.map((t) => t[0])]) addTool(tool);
  })();
  return ready;
}

function windowText({ lower, upper }) {
  const { windowWidth, windowCenter } = utilities.windowLevel.toWindowLevel(lower, upper);
  return `W ${Math.round(windowWidth)}  L ${Math.round(windowCenter)}`;
}

// orientation markers: where each edge of the screen points in the patient, measured by
// projecting the patient axes (DICOM LPS) through the viewport, so rotation and flips count
const AXES = [
  [[1, 0, 0], "L", "R"],
  [[0, 1, 0], "P", "A"],
  [[0, 0, 1], "H", "F"],
];
function markers(viewport) {
  const f = viewport.getCamera().focalPoint;
  const c = viewport.worldToCanvas(f);
  const deltas = AXES.map(([axis]) => {
    const q = viewport.worldToCanvas(f.map((x, i) => x + axis[i] * 100));
    return [q[0] - c[0], q[1] - c[1]];
  });
  const edge = (u) => {
    const scores = deltas.map((d) => d[0] * u[0] + d[1] * u[1]);
    const top = Math.max(...scores.map(Math.abs));
    return AXES.map((a, i) => [scores[i], a])
      .filter(([x]) => top > 0 && Math.abs(x) > 0.25 * top)
      .sort((a, b) => Math.abs(b[0]) - Math.abs(a[0]))
      .slice(0, 2)
      .map(([x, [, plus, minus]]) => (x > 0 ? plus : minus))
      .join("");
  };
  return { or: edge([1, 0]), ol: edge([-1, 0]), ot: edge([0, -1]), ob: edge([0, 1]) };
}
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

export async function viewerPage([studyId, seriesId], view) {
  const [study, allSeries] = await Promise.all([api(`studies/${studyId}`), api(`studies/${studyId}/series`).then(sortSeries), initCornerstone()]);
  if (!allSeries.some((s) => s.ID === seriesId)) throw new HttpError(404, "no such series in this study");
  const p = study.PatientMainDicomTags;
  const t = study.MainDicomTags;

  // a series' images, loaded once: one image ID per frame, in Instance Number order
  const loaded = new Map();
  function loadSeries(id) {
    if (!loaded.has(id)) {
      loaded.set(
        id,
        (async () => {
          const series = allSeries.find((s) => s.ID === id);
          const url = new URL(`studies/${t.StudyInstanceUID}/series/${series.MainDicomTags.SeriesInstanceUID}/`, WEB);
          const [instances, metadata] = await Promise.all([api(`series/${id}/instances`), api("metadata", {}, url)]);
          const byUid = new Map(metadata.map((m) => [first(m, "00080018"), m]));
          const slots = [];
          for (const instance of instances) {
            const uid = instance.MainDicomTags.SOPInstanceUID;
            const meta = byUid.get(uid);
            if (!meta) continue;
            const frames = Math.max(1, num(meta, "00280008", 1));
            for (let f = 1; f <= frames; f++) {
              const imageId = `wadors:${url}instances/${uid}/frames/${f}`;
              wadors.metaDataManager.add(imageId, meta);
              slots.push({ imageId, instance, meta, frame: f, frames });
            }
          }
          return { series, slots };
        })(),
      );
    }
    return loaded.get(id);
  }

  // ---- the page ----
  const grid = h("div", { class: "vgrid" });
  const position = h("span");
  const slider = h("input", { type: "range", min: "0", max: "0", value: "0", "aria-label": "Image" });
  const tagInput = h("input", { type: "search", placeholder: "Filter tags", "aria-label": "Filter tags" });
  const tagList = h("div", { class: "list" });
  const tagsPanel = h(
    "aside",
    { class: "tags", hidden: true, "aria-label": "DICOM tags" },
    h("header", {}, h("label", { class: "field" }, searchIcon(), tagInput), h("button", { class: "btn", type: "button", onclick: () => toggleTags(false) }, "Close")),
    tagList,
  );
  const toggle = (label, title, onclick, pressed = false) =>
    h("button", { class: "toggle", type: "button", title, "aria-pressed": pressed == null ? null : String(pressed), onclick }, label);
  const layoutButtons = LAYOUTS.map(([label], i) => toggle(label, `Layout ${label}`, () => setLayout(i), i === 0));
  const presetButtons = PRESETS.map(([label], i) => toggle(label, `${label} (${i + 1})`, () => setPreset(i), i === 0));
  const presetSelect = h(
    "select",
    { class: "toggle", "aria-label": "Window preset", onchange: (e) => setPreset(Number(e.target.value)) },
    PRESETS.map(([label], i) => h("option", { value: String(i) }, label)),
  );
  const invertButton = toggle("Inv", "Invert (I)", () => setInvert(!active().invert));
  const linkButton = toggle("Link", "Scroll the viewports together, by slice position (K)", () => setLink(!linked));
  const rotateButton = toggle("⟳", "Rotate 90° (E)", () => rotate(), null);
  const flipHButton = toggle("⇋", "Flip left-right", () => flip("flipHorizontal"), null);
  const flipVButton = toggle("⇵", "Flip up-down", () => flip("flipVertical"), null);
  const tagsButton = h("button", { class: "btn", type: "button", "aria-pressed": "false", title: "DICOM tags (T)", onclick: () => toggleTags() }, "Tags");
  const fullButton = h("button", { class: "btn", type: "button", title: "Full screen (F)", onclick: () => fullscreen() }, "⛶");
  const fileLink = h("a", { class: "btn", download: "", title: "Download this instance as stored" }, ".dcm");
  const toolButtons = LEFT_TOOLS.map(([tool, label, key, title]) =>
    h("button", { class: "tool", type: "button", title: `${title} (${key.toUpperCase()})`, "aria-pressed": String(tool === WindowLevelTool), onclick: () => useTool(tool) }, label),
  );
  const cineButton = h("button", { class: "tool", type: "button", title: "Play the series (Space)", "aria-pressed": "false", onclick: () => toggleCine() }, "▶");
  const clearButton = h("button", { class: "tool", type: "button", title: "Remove all measurements (Shift+Del)", onclick: () => clearMeasurements() }, "Clr");
  const rail = h("nav", { class: "rail", "aria-label": "Series of this study: click to show in the active viewport" });
  const root = h(
    "div",
    { class: "viewer" },
    rail,
    h(
      "div",
      { class: "vmain" },
      h(
        "div",
        { class: "vbar" },
        h("a", { class: "btn", href: `#/studies/${studyId}`, title: "Back to the worklist (Esc)" }, "←"),
        h("div", { class: "title" }, personName(p.PatientName), h("span", {}, t.StudyDescription || "")),
        h("div", { class: "spacer" }),
        h("div", { class: "toggles", role: "group", "aria-label": "Layout" }, layoutButtons),
        linkButton,
        h("div", { class: "toggles presets", role: "group", "aria-label": "Window presets" }, presetButtons),
        presetSelect,
        h("div", { class: "toggles", role: "group", "aria-label": "Orientation" }, invertButton, rotateButton, flipHButton, flipVButton),
        tagsButton,
        fileLink,
        fullButton,
      ),
      h("div", { class: "vbody" }, h("div", { class: "vtools", role: "toolbar", "aria-label": "Left mouse button", "aria-orientation": "vertical" }, toolButtons, h("hr"), cineButton, clearButton), grid),
      h("div", { class: "scrub" }, slider, position),
    ),
    tagsPanel,
  );
  view.replaceChildren(root);
  setStatus([plural(allSeries.length, "series", "series")], [
    ["wheel", "image"],
    ["drag", "window"],
    ["right-drag", "zoom"],
    ["shift-drag", "pan"],
    ["[ ]", "series"],
    ["1-6", "presets"],
    ["K", "link"],
    ["O", "overlays"],
    ["F", "full screen"],
    ["dbl-click", "reset"],
  ]);

  // ---- Cornerstone: one engine, one tool group, a viewport per cell ----
  ToolGroupManager.destroyToolGroup(TOOLS);
  const engine = new RenderingEngine(ENGINE);
  const tools = ToolGroupManager.createToolGroup(TOOLS);
  for (const tool of [PanTool, ZoomTool, StackScrollTool, ...LEFT_TOOLS.map((x) => x[0])]) tools.addTool(tool.toolName);
  const { MouseBindings, KeyboardBindings } = ToolEnums;
  tools.setToolActive(WindowLevelTool.toolName, { bindings: [{ mouseButton: MouseBindings.Primary }] });
  tools.setToolActive(PanTool.toolName, { bindings: [{ mouseButton: MouseBindings.Auxiliary }, { mouseButton: MouseBindings.Primary, modifierKey: KeyboardBindings.Shift }] });
  tools.setToolActive(ZoomTool.toolName, { bindings: [{ mouseButton: MouseBindings.Secondary }] });
  tools.setToolActive(StackScrollTool.toolName, { bindings: [{ mouseButton: MouseBindings.Wheel }] });
  for (const [tool] of LEFT_TOOLS.slice(1)) tools.setToolPassive(tool.toolName);
  // measurements belong to this viewing only
  annotation.state.removeAllAnnotations();

  const cells = [];
  let current = 0;
  let linked = false;
  let syncing = false;
  let overlaysOn = true;
  let leftTool = WindowLevelTool;
  const active = () => cells[current];

  function makeCell(i) {
    const element = h("div", { class: "cs" });
    const ov = Object.fromEntries(["tl", "tr", "bl", "br", "ol", "or", "ot", "ob"].map((k) => [k, h("div", { class: `ov ${k}` })]));
    const note = h("div", { class: "note" });
    const wrap = h("div", { class: "cell", tabindex: "0" }, element, Object.values(ov), note);
    grid.append(wrap);
    const cell = { i, id: `vp${i}`, element, ov, note, wrap, seriesId: null, series: null, slots: [], index: 0, preset: 0, invert: false, rotation: 0, fitZoom: null, failed: new Set(), playing: false };
    engine.enableElement({ viewportId: cell.id, type: Enums.ViewportType.STACK, element, defaultOptions: { background: [0, 0, 0] } });
    tools.addViewport(cell.id, ENGINE);
    cell.viewport = engine.getViewport(cell.id);
    wrap.addEventListener("pointerdown", () => activate(cell.i), true);
    wrap.addEventListener("contextmenu", (e) => e.preventDefault());
    wrap.addEventListener("dblclick", () => {
      cell.viewport.resetCamera();
      cell.rotation = 0;
      setPreset(0, cell);
    });
    element.addEventListener(Enums.Events.STACK_NEW_IMAGE, (e) => {
      cell.index = e.detail.imageIdIndex;
      overlays(cell);
      if (linked && !syncing && cell === active()) syncFrom(cell);
    });
    element.addEventListener(Enums.Events.VOI_MODIFIED, () => {
      if (cell.preset > 0) {
        const w = PRESETS[cell.preset][1];
        const voi = cell.viewport.getProperties()?.voiRange;
        const want = utilities.windowLevel.toLowHighRange(w.width, w.center);
        if (voi && (Math.abs(voi.lower - want.lower) > 0.5 || Math.abs(voi.upper - want.upper) > 0.5)) {
          cell.preset = -1;
          if (cell === active()) syncToolbar();
        }
      }
      overlays(cell);
    });
    element.addEventListener(Enums.Events.CAMERA_MODIFIED, () => overlays(cell));
    return cell;
  }

  async function show(cell, id, at = null) {
    if (cell.playing) toggleCine(false, cell);
    cell.seriesId = id;
    cell.note.textContent = "loading…";
    const { series, slots } = await loadSeries(id);
    if (cell.seriesId !== id) return;
    cell.series = series;
    cell.slots = slots;
    cell.failed = new Set();
    cell.preset = 0;
    cell.rotation = 0;
    markRail();
    if (!slots.length) {
      cell.note.textContent = "This series has no images.";
      return;
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
    overlays(cell);
    if (cell === active()) syncToolbar();
  }

  function overlays(cell) {
    const { ov, slots, series } = cell;
    if (!slots.length || !series) return;
    const slot = slots[cell.index];
    const st = series.MainDicomTags;
    const pos = tag(slot.meta, "00200032");
    const thickness = num(slot.meta, "00180050", NaN);
    const columns = num(slot.meta, "00280011", 0), rows = num(slot.meta, "00280010", 0);
    ov.tl.textContent = `${personName(p.PatientName)}\n${p.PatientID || ""}  ${sexAge(p.PatientSex, p.PatientBirthDate, t.StudyDate)}\n${dicomDate(t.StudyDate)} ${dicomTime(t.StudyTime)}`;
    ov.tr.textContent = `${t.StudyDescription || ""}\n${st.Modality || ""} ${st.SeriesNumber ?? ""}  ${st.SeriesDescription || ""}\n${t.AccessionNumber ? `ACC ${t.AccessionNumber}` : ""}`;
    ov.br.textContent = [pos ? `Z ${Number(pos[2]).toFixed(1)} mm` : null, isFinite(thickness) ? `TH ${thickness} mm` : null, rows ? `${columns} × ${rows}` : null].filter(Boolean).join("\n");
    const voi = cell.viewport.getProperties()?.voiRange;
    let zoom = 1;
    try {
      if (cell.fitZoom) zoom = cell.viewport.getZoom() / cell.fitZoom;
    } catch {}
    ov.bl.textContent = [
      `IM ${cell.index + 1} / ${slots.length}${slot.frames > 1 ? `  FR ${slot.frame}` : ""}`,
      voi ? windowText(voi) : "",
      Math.abs(zoom - 1) > 0.01 ? `ZOOM ${Math.round(zoom * 100)}%` : "",
    ].filter(Boolean).join("\n");
    try {
      for (const [k, v] of Object.entries(markers(cell.viewport))) ov[k].textContent = v;
    } catch {}
    cell.note.textContent = cell.failed.has(slot.imageId) ? "Could not decode this image." : "";
    if (cell === active()) {
      slider.max = String(Math.max(slots.length - 1, 0));
      slider.value = String(cell.index);
      position.textContent = `${cell.index + 1} / ${slots.length}`;
      fileLink.href = new URL(`instances/${slot.instance.ID}/file`, ROOT).href;
      if (!tagsPanel.hidden) renderTags();
    }
  }

  function frameActive() {
    cells.forEach((c) => c.wrap.classList.toggle("active", c.i === current && cells.length > 1));
  }

  function activate(i) {
    if (i === current) return;
    current = i;
    frameActive();
    markRail();
    syncToolbar();
    overlays(active());
    if (active().seriesId) history.replaceState(null, "", `#/view/${studyId}/${active().seriesId}`);
  }

  function syncToolbar() {
    const cell = active();
    presetButtons.forEach((b, j) => b.setAttribute("aria-pressed", String(j === cell.preset)));
    presetSelect.value = String(Math.max(cell.preset, 0));
    invertButton.setAttribute("aria-pressed", String(cell.invert));
    cineButton.setAttribute("aria-pressed", String(cell.playing));
    cineButton.textContent = cell.playing ? "■" : "▶";
  }

  function markRail() {
    const shown = new Set(cells.map((c) => c.seriesId));
    for (const a of rail.querySelectorAll(".thumb")) {
      a.setAttribute("aria-current", String(a.dataset.series === active()?.seriesId));
      a.classList.toggle("shown", shown.has(a.dataset.series));
    }
  }

  // the layout: new viewports show the series after those already shown
  async function setLayout(i) {
    const [, cols, rows] = LAYOUTS[i];
    layoutButtons.forEach((b, j) => b.setAttribute("aria-pressed", String(j === i)));
    grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
    grid.style.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`;
    const n = cols * rows;
    while (cells.length > n) {
      const cell = cells.pop();
      if (cell.playing) toggleCine(false, cell);
      try {
        toolUtilities.stackPrefetch.disable(cell.element);
      } catch {}
      tools.removeViewports(ENGINE, cell.id);
      engine.disableElement(cell.id);
      cell.wrap.remove();
    }
    const fresh = [];
    while (cells.length < n) {
      const cell = makeCell(cells.length);
      cells.push(cell);
      fresh.push(cell);
    }
    refit();
    if (current >= cells.length) current = 0;
    frameActive();
    const start = allSeries.findIndex((s) => s.ID === cells[0].seriesId);
    await Promise.all(fresh.map((cell) => show(cell, allSeries[(Math.max(start, 0) + cell.i) % allSeries.length].ID)));
    markRail();
    if (linked) syncFrom(active());
  }

  // linked scrolling: the slice of each other viewport nearest to this one's, along its normal
  function syncFrom(source) {
    if (!source.slots.length) return;
    const slot = source.slots[source.index];
    const frame = first(slot.meta, "00200052", null);
    const pos = tag(slot.meta, "00200032");
    if (!pos) return;
    syncing = true;
    for (const cell of cells) {
      if (cell === source || !cell.slots.length) continue;
      if (frame && first(cell.slots[0].meta, "00200052", null) !== frame) continue;
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
    syncing = false;
  }
  function setLink(on) {
    linked = on;
    linkButton.setAttribute("aria-pressed", String(on));
    if (on) syncFrom(active());
  }

  function go(i) {
    const cell = active();
    const n = Math.max(0, Math.min(cell.slots.length - 1, i));
    if (cell.slots.length && n !== cell.viewport.getCurrentImageIdIndex()) cell.viewport.setImageIdIndex(n);
  }

  function setPreset(i, cell = active()) {
    cell.preset = i;
    const w = PRESETS[i][1];
    if (w) cell.viewport.setProperties({ voiRange: utilities.windowLevel.toLowHighRange(w.width, w.center) });
    else {
      cell.viewport.resetProperties();
      cell.viewport.setProperties({ invert: cell.invert });
    }
    cell.viewport.render();
    if (cell === active()) syncToolbar();
  }
  function setInvert(on, cell = active()) {
    cell.invert = on;
    cell.viewport.setProperties({ invert: on });
    cell.viewport.render();
    syncToolbar();
  }
  function rotate(cell = active()) {
    cell.rotation = (cell.rotation + 90) % 360;
    cell.viewport.setViewPresentation({ rotation: cell.rotation });
    cell.viewport.render();
    overlays(cell);
  }
  // as seen on screen: on an image turned a quarter, left-right is the image's up-down
  function flip(which, cell = active()) {
    if (cell.rotation % 180 === 90) which = which === "flipHorizontal" ? "flipVertical" : "flipHorizontal";
    cell.viewport.setCamera({ [which]: !cell.viewport.getCamera()[which] });
    cell.viewport.render();
    overlays(cell);
  }
  function fullscreen() {
    if (document.fullscreenElement) document.exitFullscreen();
    else root.requestFullscreen?.();
  }
  function toggleOverlays() {
    overlaysOn = !overlaysOn;
    grid.classList.toggle("no-overlays", !overlaysOn);
  }

  function useTool(tool) {
    if (tool === leftTool) return;
    tools.setToolPassive(leftTool.toolName);
    tools.setToolActive(tool.toolName, { bindings: [{ mouseButton: MouseBindings.Primary }] });
    leftTool = tool;
    toolButtons.forEach((b, i) => b.setAttribute("aria-pressed", String(LEFT_TOOLS[i][0] === tool)));
    grid.classList.toggle("measuring", tool !== WindowLevelTool);
  }
  function clearMeasurements() {
    annotation.state.removeAllAnnotations();
    cells.forEach((c) => c.viewport.render());
  }
  function toggleCine(on, cell = active()) {
    on ??= !cell.playing;
    if (cell.slots.length < 2) return;
    cell.playing = on;
    if (on) toolUtilities.cine.playClip(cell.element, { framesPerSecond: 15, loop: true });
    else toolUtilities.cine.stopClip(cell.element);
    if (cell === active()) syncToolbar();
  }

  function toggleTags(showTags = tagsPanel.hidden) {
    tagsPanel.hidden = !showTags;
    tagsButton.setAttribute("aria-pressed", String(showTags));
    if (showTags) {
      renderTags();
      tagInput.focus();
    } else active().wrap.focus();
  }
  let tagsFor = null;
  function renderTags() {
    const cell = active();
    if (!cell.slots.length) return;
    const query = tagInput.value.toLowerCase();
    const key = `${cell.slots[cell.index].imageId} ${query}`;
    if (tagsFor === key) return;
    tagsFor = key;
    tagList.replaceChildren(h("table", {}, h("tbody", {}, tagRows(cell.slots[cell.index].meta, query))));
  }
  tagInput.addEventListener("input", renderTags);

  // the rail: a click shows the series in the active viewport
  const showInActive = (id) => show(active(), id).then(() => history.replaceState(null, "", `#/view/${studyId}/${id}`));
  rail.replaceChildren(
    ...allSeries.map((s) => {
      const a = seriesThumb(s, { id: studyId, uid: t.StudyInstanceUID }, false);
      a.dataset.series = s.ID;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        showInActive(s.ID);
      });
      return a;
    }),
  );

  const onLoadFailed = (e) => {
    const id = e.detail?.imageId;
    for (const cell of cells) {
      if (cell.slots.some((s) => s.imageId === id)) {
        cell.failed.add(id);
        overlays(cell);
      }
    }
  };
  eventTarget.addEventListener(Enums.Events.IMAGE_LOAD_FAILED, onLoadFailed);
  slider.addEventListener("input", () => go(Number(slider.value)));
  // a resize fits each image again: zoom is shown relative to that fit
  function refit() {
    engine.resize(true, false);
    for (const cell of cells) {
      try {
        cell.fitZoom = cell.viewport.getZoom();
      } catch {}
      overlays(cell);
    }
  }
  const resize = new ResizeObserver(refit);
  resize.observe(grid);

  const otherSeries = (step) => {
    const at = allSeries.findIndex((s) => s.ID === active().seriesId);
    const next = allSeries[at + step];
    if (next) showInActive(next.ID);
  };

  const keys = (e) => {
    if ((isTyping(e) && e.target !== slider) || e.target === tagInput) {
      if (e.key === "Escape" && e.target === tagInput) toggleTags(false);
      return;
    }
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1, PageDown: 10, PageUp: -10 }[e.key];
    const tool = LEFT_TOOLS.find((x) => x[2] === e.key);
    if (step) {
      e.preventDefault();
      go(active().index + step);
    } else if (e.key === "Home") go(0);
    else if (e.key === "End") go(active().slots.length - 1);
    else if (e.key === "i") setInvert(!active().invert);
    else if (e.key === "e") rotate();
    else if (e.key === "k") setLink(!linked);
    else if (e.key === "o") toggleOverlays();
    else if (e.key === "f") fullscreen();
    else if (e.key === " ") {
      e.preventDefault();
      toggleCine();
    } else if (e.key === "Delete" && e.shiftKey) clearMeasurements();
    else if (tool) useTool(tool[0]);
    else if (e.key === "t") {
      e.preventDefault();
      toggleTags();
    } else if (e.key === "[") otherSeries(-1);
    else if (e.key === "]") otherSeries(1);
    else if (e.key >= "1" && e.key <= String(PRESETS.length)) setPreset(Number(e.key) - 1);
    else if (e.key === "Escape") {
      if (!tagsPanel.hidden) toggleTags(false);
      else if (!document.fullscreenElement) location.hash = `#/studies/${studyId}`;
    }
  };
  document.addEventListener("keydown", keys);

  // one viewport to start with, showing the series asked for
  grid.style.gridTemplateColumns = grid.style.gridTemplateRows = "minmax(0, 1fr)";
  cells.push(makeCell(0));
  engine.resize(true, false);
  await show(cells[0], seriesId);
  cells[0].wrap.focus();

  return {
    cleanup: () => {
      document.removeEventListener("keydown", keys);
      eventTarget.removeEventListener(Enums.Events.IMAGE_LOAD_FAILED, onLoadFailed);
      resize.disconnect();
      for (const cell of cells) {
        if (cell.playing) toggleCine(false, cell);
        try {
          toolUtilities.stackPrefetch.disable(cell.element);
        } catch {}
      }
      if (document.fullscreenElement) document.exitFullscreen();
      ToolGroupManager.destroyToolGroup(TOOLS);
      engine.destroy();
    },
  };
}
