// The image viewer: a Cornerstone3D stack viewport. Frames come over WADO-RS in the transfer
// syntax they are stored in (JPEG-LS by default) and are decoded in web workers by
// dicom-image-loader's codecs; the metadata is fetched once per series and handed to the
// loader, so no image needs a request of its own for it.

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
  EllipticalROITool,
  annotation,
  Enums as ToolEnums,
  utilities as toolUtilities,
} from "@cornerstonejs/tools";
import { h, searchIcon, HttpError, api, personName, dicomDate, dicomTime, sexAge, plural, setStatus, isTyping, ROOT, WEB } from "./util.js";
import { sortSeries, seriesThumb } from "./thumbs.js";
import { first, tag, num } from "./pixels.js";
import { tagRows } from "./tags.js";

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
    for (const tool of [WindowLevelTool, PanTool, ZoomTool, StackScrollTool, ...LEFT_TOOLS.map((t) => t[0])]) addTool(tool);
  })();
  return ready;
}

// what the left mouse button does: [tool, label, key, title]
const LEFT_TOOLS = [
  [WindowLevelTool, "W/L", "w", "Window/level"],
  [LengthTool, "Len", "l", "Length"],
  [ProbeTool, "HU", "h", "Pixel value"],
  [AngleTool, "Ang", "a", "Angle"],
  [EllipticalROITool, "ROI", "r", "Elliptical ROI: area, mean, std"],
];

const PRESETS = [
  ["Auto", null],
  ["Soft", { center: 40, width: 400 }],
  ["Liver", { center: 60, width: 160 }],
  ["Lung", { center: -600, width: 1500 }],
  ["Bone", { center: 400, width: 1800 }],
  ["Brain", { center: 40, width: 80 }],
];

const ENGINE = "pacs-rs";
const VIEWPORT = "stack";
const TOOLS = "pacs-rs-tools";

function windowText({ lower, upper }) {
  const { windowWidth, windowCenter } = utilities.windowLevel.toWindowLevel(lower, upper);
  return `W ${Math.round(windowWidth)}  L ${Math.round(windowCenter)}`;
}

export async function viewerPage([studyId, seriesId], view) {
  const [study, allSeries, instances] = await Promise.all([
    api(`studies/${studyId}`),
    api(`studies/${studyId}/series`).then(sortSeries),
    api(`series/${seriesId}/instances`),
    initCornerstone(),
  ]);
  const series = allSeries.find((s) => s.ID === seriesId);
  if (!series) throw new HttpError(404, "no such series in this study");
  const st = series.MainDicomTags;
  const p = study.PatientMainDicomTags;
  const t = study.MainDicomTags;
  const seriesUrl = new URL(`studies/${t.StudyInstanceUID}/series/${st.SeriesInstanceUID}/`, WEB);
  const metadata = await api(`metadata`, {}, seriesUrl);
  const metaByUid = new Map(metadata.map((m) => [first(m, "00080018"), m]));

  // one image ID per frame, in the REST API's order (Instance Number)
  const slots = [];
  for (const instance of instances) {
    const uid = instance.MainDicomTags.SOPInstanceUID;
    const meta = metaByUid.get(uid);
    if (!meta) continue;
    const frames = Math.max(1, num(meta, "00280008", 1));
    for (let f = 1; f <= frames; f++) {
      const imageId = `wadors:${seriesUrl}instances/${uid}/frames/${f}`;
      wadors.metaDataManager.add(imageId, meta);
      slots.push({ imageId, instance, meta, frame: f, frames });
    }
  }

  const element = h("div", { class: "cs" });
  const ov = { tl: h("div", { class: "ov tl" }), tr: h("div", { class: "ov tr" }), bl: h("div", { class: "ov bl" }), br: h("div", { class: "ov br" }) };
  const note = h("div", { class: "note" });
  const stage = h("div", { class: "stage", tabindex: "0", "aria-label": "Image. Wheel or arrow keys change the image; drag changes the window." }, element, Object.values(ov), note);
  const slider = h("input", { type: "range", min: "0", max: String(Math.max(slots.length - 1, 0)), value: "0", "aria-label": "Image" });
  const position = h("span");
  const tagInput = h("input", { type: "search", placeholder: "Filter tags", "aria-label": "Filter tags" });
  const tagList = h("div", { class: "list" });
  const tagsPanel = h(
    "aside",
    { class: "tags", hidden: true, "aria-label": "DICOM tags" },
    h("header", {}, h("label", { class: "field" }, searchIcon(), tagInput), h("button", { class: "btn", type: "button", onclick: () => toggleTags(false) }, "Close")),
    tagList,
  );
  const presetButtons = PRESETS.map(([label, w], i) =>
    h("button", { class: "toggle", type: "button", title: `${label} (${i + 1})`, "aria-pressed": String(i === 0), onclick: () => setPreset(i) }, label),
  );
  const presetSelect = h(
    "select",
    { class: "toggle", "aria-label": "Window preset", onchange: (e) => setPreset(Number(e.target.value)) },
    PRESETS.map(([label], i) => h("option", { value: String(i) }, label)),
  );
  const invertButton = h("button", { class: "toggle", type: "button", "aria-pressed": "false", title: "Invert (I)", onclick: () => setInvert(!invert) }, "Inv");
  const tagsButton = h("button", { class: "btn", type: "button", "aria-pressed": "false", title: "DICOM tags (T)", onclick: () => toggleTags() }, "Tags");
  const toolButtons = LEFT_TOOLS.map(([tool, label, key, title]) =>
    h("button", { class: "tool", type: "button", title: `${title} (${key.toUpperCase()})`, "aria-pressed": String(tool === WindowLevelTool), onclick: () => useTool(tool) }, label),
  );
  const cineButton = h("button", { class: "tool", type: "button", title: "Play the series (Space)", "aria-pressed": "false", onclick: () => toggleCine() }, "▶");
  const clearButton = h("button", { class: "tool", type: "button", title: "Remove all measurements (Shift+Del)", onclick: () => clearMeasurements() }, "Clr");
  const fileLink = h("a", { class: "btn", download: "", title: "Download this instance as stored" }, h("span", { class: "txt" }, "Download "), ".dcm");
  const at = allSeries.indexOf(series);
  const otherSeries = (step) => {
    const next = allSeries[at + step];
    if (next) location.replace(`#/view/${studyId}/${next.ID}`);
  };

  view.replaceChildren(
    h(
      "div",
      { class: "viewer" },
      h("nav", { class: "rail", "aria-label": "Series of this study" }, allSeries.map((s) => seriesThumb(s, { id: studyId, uid: t.StudyInstanceUID }, s.ID === seriesId))),
      h(
        "div",
        { class: "vmain" },
        h(
          "div",
          { class: "vbar" },
          h("a", { class: "btn", href: `#/studies/${studyId}`, title: "Back to the worklist (Esc)" }, "←", h("span", { class: "txt" }, " Worklist")),
          h("div", { class: "title" }, personName(p.PatientName), h("span", {}, `${t.StudyDescription || ""} · ${st.Modality || ""} ${st.SeriesNumber ?? ""} ${st.SeriesDescription || ""}`)),
          h("div", { class: "spacer" }),
          h("div", { class: "toggles presets", role: "group", "aria-label": "Window presets" }, presetButtons),
          presetSelect,
          invertButton,
          tagsButton,
          fileLink,
        ),
        h("div", { class: "vbody" }, h("div", { class: "vtools", role: "toolbar", "aria-label": "Left mouse button", "aria-orientation": "vertical" }, toolButtons, h("hr"), cineButton, clearButton), stage),
        h("div", { class: "scrub" }, slider, position),
      ),
      tagsPanel,
    ),
  );
  setStatus([h("b", {}, `${st.Modality || ""} ${st.SeriesNumber ?? ""}`), plural(slots.length, "image"), `series ${at + 1} of ${allSeries.length}`], [
    ["wheel", "image"],
    ["drag", "window"],
    ["right-drag", "zoom"],
    ["shift-drag", "pan"],
    ["[ ]", "series"],
    ["W L H A R", "tools"],
    ["space", "cine"],
    ["1-6", "presets"],
    ["dbl-click", "reset"],
  ]);
  requestAnimationFrame(() => view.querySelector('.rail [aria-current="true"]')?.scrollIntoView({ block: "nearest" }));

  if (!slots.length) {
    note.textContent = "This series has no images.";
    return {};
  }

  // one rendering engine and one tool group at a time
  ToolGroupManager.destroyToolGroup(TOOLS);
  const engine = new RenderingEngine(ENGINE);
  engine.enableElement({ viewportId: VIEWPORT, type: Enums.ViewportType.STACK, element, defaultOptions: { background: [0, 0, 0] } });
  const viewport = engine.getViewport(VIEWPORT);
  const tools = ToolGroupManager.createToolGroup(TOOLS);
  for (const tool of [WindowLevelTool, PanTool, ZoomTool, StackScrollTool, ...LEFT_TOOLS.slice(1).map((t) => t[0])]) tools.addTool(tool.toolName);
  const { MouseBindings, KeyboardBindings } = ToolEnums;
  tools.setToolActive(WindowLevelTool.toolName, { bindings: [{ mouseButton: MouseBindings.Primary }] });
  tools.setToolActive(PanTool.toolName, { bindings: [{ mouseButton: MouseBindings.Auxiliary }, { mouseButton: MouseBindings.Primary, modifierKey: KeyboardBindings.Shift }] });
  tools.setToolActive(ZoomTool.toolName, { bindings: [{ mouseButton: MouseBindings.Secondary }] });
  tools.setToolActive(StackScrollTool.toolName, { bindings: [{ mouseButton: MouseBindings.Wheel }] });
  for (const [tool] of LEFT_TOOLS.slice(1)) tools.setToolPassive(tool.toolName);
  tools.addViewport(VIEWPORT, ENGINE);
  // measurements belong to this series' viewing only
  annotation.state.removeAllAnnotations();

  let leftTool = WindowLevelTool;
  function useTool(tool) {
    if (tool === leftTool) return;
    tools.setToolPassive(leftTool.toolName);
    tools.setToolActive(tool.toolName, { bindings: [{ mouseButton: MouseBindings.Primary }] });
    leftTool = tool;
    toolButtons.forEach((b, i) => b.setAttribute("aria-pressed", String(LEFT_TOOLS[i][0] === tool)));
    stage.style.cursor = tool === WindowLevelTool ? "" : "default";
  }
  function clearMeasurements() {
    annotation.state.removeAllAnnotations();
    viewport.render();
  }
  let playing = false;
  function toggleCine(on = !playing) {
    if (slots.length < 2) return;
    playing = on;
    if (on) toolUtilities.cine.playClip(element, { framesPerSecond: 15, loop: true });
    else toolUtilities.cine.stopClip(element);
    cineButton.setAttribute("aria-pressed", String(on));
    cineButton.textContent = on ? "■" : "▶";
  }

  let index = Math.floor(slots.length / 2);
  let invert = false;
  let preset = 0;
  let failed = new Set();
  let fitZoom = null; // Cornerstone's zoom when the image fits the viewport

  function overlays() {
    const slot = slots[index];
    slider.value = String(index);
    position.textContent = `${index + 1} / ${slots.length}`;
    fileLink.href = new URL(`instances/${slot.instance.ID}/file`, ROOT).href;
    const pos = tag(slot.meta, "00200032");
    const thickness = num(slot.meta, "00180050", NaN);
    const columns = num(slot.meta, "00280011", 0), rows = num(slot.meta, "00280010", 0);
    ov.tl.textContent = `${personName(p.PatientName)}\n${p.PatientID || ""}  ${sexAge(p.PatientSex, p.PatientBirthDate, t.StudyDate)}\n${dicomDate(t.StudyDate)} ${dicomTime(t.StudyTime)}`;
    ov.tr.textContent = `${t.StudyDescription || ""}\n${st.Modality || ""} ${st.SeriesNumber ?? ""}  ${st.SeriesDescription || ""}\n${t.AccessionNumber ? `ACC ${t.AccessionNumber}` : ""}`;
    ov.br.textContent = [pos ? `Z ${Number(pos[2]).toFixed(1)} mm` : null, isFinite(thickness) ? `TH ${thickness} mm` : null, rows ? `${columns} × ${rows}` : null]
      .filter(Boolean)
      .join("\n");
    const voi = viewport.getProperties()?.voiRange;
    let zoom = 1;
    try {
      if (fitZoom) zoom = viewport.getZoom() / fitZoom;
    } catch {}
    ov.bl.textContent = [
      `IM ${index + 1} / ${slots.length}${slot.frames > 1 ? `  FR ${slot.frame}` : ""}`,
      voi ? windowText(voi) : "",
      Math.abs(zoom - 1) > 0.01 ? `ZOOM ${Math.round(zoom * 100)}%` : "",
    ]
      .filter(Boolean)
      .join("\n");
    note.textContent = failed.has(slot.imageId) ? "Could not decode this image." : "";
    if (!tagsPanel.hidden) renderTags();
  }

  function go(i) {
    const n = Math.max(0, Math.min(slots.length - 1, i));
    if (n !== viewport.getCurrentImageIdIndex()) viewport.setImageIdIndex(n);
  }

  function setPreset(i) {
    preset = i;
    const w = PRESETS[i][1];
    if (w) viewport.setProperties({ voiRange: utilities.windowLevel.toLowHighRange(w.width, w.center) });
    else {
      viewport.resetProperties();
      viewport.setProperties({ invert });
    }
    viewport.render();
    markPreset(i);
  }
  function markPreset(i) {
    presetButtons.forEach((b, j) => b.setAttribute("aria-pressed", String(j === i)));
    presetSelect.value = String(Math.max(i, 0));
  }
  function setInvert(on) {
    invert = on;
    invertButton.setAttribute("aria-pressed", String(on));
    viewport.setProperties({ invert: on });
    viewport.render();
  }

  function toggleTags(show = tagsPanel.hidden) {
    tagsPanel.hidden = !show;
    tagsButton.setAttribute("aria-pressed", String(show));
    if (show) {
      renderTags();
      tagInput.focus();
    } else stage.focus();
  }
  let tagsFor = null;
  function renderTags() {
    const query = tagInput.value.toLowerCase();
    const key = `${slots[index].imageId} ${query}`;
    if (tagsFor === key) return;
    tagsFor = key;
    tagList.replaceChildren(h("table", {}, h("tbody", {}, tagRows(slots[index].meta, query))));
  }
  tagInput.addEventListener("input", renderTags);

  // Cornerstone tells us what changed; the overlays follow
  const onNewImage = (e) => {
    index = e.detail.imageIdIndex;
    overlays();
  };
  const onVoi = () => {
    // a drag changes the window away from the preset
    if (preset > 0) {
      const w = PRESETS[preset][1];
      const voi = viewport.getProperties()?.voiRange;
      const want = utilities.windowLevel.toLowHighRange(w.width, w.center);
      if (voi && (Math.abs(voi.lower - want.lower) > 0.5 || Math.abs(voi.upper - want.upper) > 0.5)) markPreset((preset = -1));
    }
    overlays();
  };
  const onLoadFailed = (e) => {
    const id = e.detail?.imageId;
    if (!id || !slots.some((s) => s.imageId === id)) return;
    failed.add(id);
    overlays();
  };
  element.addEventListener(Enums.Events.STACK_NEW_IMAGE, onNewImage);
  element.addEventListener(Enums.Events.VOI_MODIFIED, onVoi);
  element.addEventListener(Enums.Events.CAMERA_MODIFIED, overlays);
  eventTarget.addEventListener(Enums.Events.IMAGE_LOAD_FAILED, onLoadFailed);

  slider.addEventListener("input", () => go(Number(slider.value)));
  stage.addEventListener("contextmenu", (e) => e.preventDefault());
  stage.addEventListener("pointerdown", () => stage.focus());
  stage.addEventListener("dblclick", () => {
    viewport.resetCamera();
    setPreset(0);
  });

  const keys = (e) => {
    if ((isTyping(e) && e.target !== slider) || e.target === tagInput) {
      if (e.key === "Escape" && e.target === tagInput) toggleTags(false);
      return;
    }
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1, PageDown: 10, PageUp: -10 }[e.key];
    if (step) {
      e.preventDefault();
      go(index + step);
    } else if (e.key === "Home") go(0);
    else if (e.key === "End") go(slots.length - 1);
    else if (e.key === "i") setInvert(!invert);
    else if (e.key === " ") {
      e.preventDefault();
      toggleCine();
    } else if (e.key === "Delete" && e.shiftKey) clearMeasurements();
    else if (LEFT_TOOLS.some((x) => x[2] === e.key)) useTool(LEFT_TOOLS.find((x) => x[2] === e.key)[0]);
    else if (e.key === "t") {
      e.preventDefault();
      toggleTags();
    } else if (e.key === "[") otherSeries(-1);
    else if (e.key === "]") otherSeries(1);
    else if (e.key >= "1" && e.key <= String(PRESETS.length)) setPreset(Number(e.key) - 1);
    else if (e.key === "Escape") {
      if (!tagsPanel.hidden) toggleTags(false);
      else location.hash = `#/studies/${studyId}`;
    }
  };
  document.addEventListener("keydown", keys);

  note.textContent = "loading…";
  try {
    await viewport.setStack(
      slots.map((s) => s.imageId),
      index,
    );
  } catch (e) {
    failed.add(slots[index].imageId);
  }
  viewport.render();
  try {
    fitZoom = viewport.getZoom();
  } catch {}
  toolUtilities.stackPrefetch.enable(element);
  overlays();
  stage.focus();

  return {
    cleanup: () => {
      document.removeEventListener("keydown", keys);
      if (playing) toggleCine(false);
      eventTarget.removeEventListener(Enums.Events.IMAGE_LOAD_FAILED, onLoadFailed);
      try {
        toolUtilities.stackPrefetch.disable(element);
      } catch {}
      ToolGroupManager.destroyToolGroup(TOOLS);
      engine.destroy();
    },
  };
}
