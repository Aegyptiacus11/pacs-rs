// pacs-rs web interface: a worklist with a preview pane, a patient list, uploads and (loaded
// on demand) the Cornerstone3D viewer, over the server's own REST API and DICOMweb. DICOM
// values are untrusted text: they reach the page only through textContent, never innerHTML.

import "./style.css";
import { h, searchIcon, HttpError, api, toast, confirmDelete, copyable, personName, parseDate, dicomDate, dicomTime, sexAge, bytes, plural, longDate, setStatus, isTyping, config, ROOT, WEB } from "./util.js";
import { kv, sortSeries, seriesThumb } from "./thumbs.js";

const view = document.getElementById("view");

// ---------- shell ----------

function setTab(name) {
  for (const a of document.querySelectorAll(".tabs a")) {
    if (a.dataset.tab === name) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
}

async function checkHealth() {
  const el = document.getElementById("health");
  let ok = false;
  try {
    ok = (await fetch(new URL("healthz", ROOT), { cache: "no-store" })).ok;
  } catch {}
  el.className = `health ${ok ? "ok" : "down"}`;
  el.querySelector("span").textContent = ok ? `Online ${config.version}` : "Offline";
  el.title = ok ? `pacs-rs ${config.version} is up` : "The server does not answer";
}
checkHealth();
setInterval(checkHealth, 30000);

// ---------- the studies, from the REST API and QIDO-RS together ----------

// REST gives public IDs, QIDO gives modalities and counts; they meet on Study Instance UID
const store = { studies: null, pending: null };

async function everyPage(fetchPage, size) {
  const out = [];
  for (let at = 0; ; at += size) {
    const page = await fetchPage(at, size);
    out.push(...page);
    if (page.length < size) return out;
  }
}

function loadStudies() {
  if (store.studies) return Promise.resolve(store.studies);
  store.pending ??= Promise.all([
    everyPage((at, n) => api(`studies?expand&since=${at}&limit=${n}`), 500),
    everyPage((at, n) => api(`studies?offset=${at}&limit=${n}`, { headers: { Accept: "application/dicom+json" } }, WEB), 1000),
  ])
    .then(([rest, qido]) => {
      const byUid = new Map(qido.map((q) => [q["0020000D"]?.Value?.[0], q]));
      store.studies = rest.map((s) => {
        const t = s.MainDicomTags, p = s.PatientMainDicomTags, q = byUid.get(t.StudyInstanceUID) || {};
        return {
          id: s.ID,
          patient: s.ParentPatient,
          uid: t.StudyInstanceUID,
          date: t.StudyDate || "",
          time: t.StudyTime || "",
          description: t.StudyDescription || "",
          accession: t.AccessionNumber || "",
          studyId: t.StudyID || "",
          referring: t.ReferringPhysicianName || "",
          name: p.PatientName || "",
          patientId: p.PatientID || "",
          birth: p.PatientBirthDate || "",
          sex: p.PatientSex || "",
          modalities: q["00080061"]?.Value || [],
          series: Number(q["00201206"]?.Value?.[0] ?? s.Series.length),
          instances: Number(q["00201208"]?.Value?.[0] ?? 0),
          updated: s.LastUpdate || "",
        };
      });
      return store.studies;
    })
    .finally(() => (store.pending = null));
  return store.pending;
}
function invalidate() {
  store.studies = null;
}

// ---------- router ----------

let page = null; // { name, cleanup?, update?(args) }
const routes = [
  [/^\/studies(?:\/([\w-]+))?$/, "studies", worklistPage],
  [/^\/patients$/, "patients", patientsPage],
  [/^\/view\/([\w-]+)\/([\w-]+)$/, "studies", viewerPage],
];

async function route() {
  const [path, query = ""] = (location.hash.replace(/^#/, "") || "/studies").split("?");
  const params = new URLSearchParams(query);
  for (const [pattern, tab, render] of routes) {
    const match = pattern.exec(path);
    if (!match) continue;
    setTab(tab);
    if (page?.render === render && page.update) return page.update(match.slice(1), params);
    page?.cleanup?.();
    page = null;
    view.replaceChildren(h("div", { class: "bar-load" }));
    try {
      page = (await render(match.slice(1), params)) || {};
      page.render = render;
    } catch (e) {
      view.replaceChildren(failure(e));
      setStatus();
    }
    return;
  }
  location.replace("#/studies");
}
addEventListener("hashchange", route);

function failure(e) {
  const missing = e instanceof HttpError && e.status === 404;
  return h(
    "div",
    { class: "nothing" },
    h("b", {}, missing ? "Not found" : "Something went wrong"),
    missing ? "It may have been deleted." : String(e.message || e),
    h("div", {}, h("a", { class: "btn", href: "#/studies" }, "Back to the worklist")),
  );
}

function emptyArchive() {
  return h(
    "div",
    { class: "nothing" },
    h("b", {}, "The archive is empty"),
    "Drop DICOM files or folders anywhere on this page, or send them with STOW-RS or POST /instances.",
    h(
      "div",
      {},
      h("button", { class: "btn primary", type: "button", onclick: () => pick("files") }, "Upload files"),
      h("button", { class: "btn", type: "button", onclick: () => pick("folder") }, "Upload a folder"),
    ),
  );
}

// rows that the arrow keys walk, the selected one marked with aria-selected
function moveSelection(tbody, step) {
  const rows = [...tbody.querySelectorAll("tr[data-id]")];
  if (!rows.length) return null;
  const at = rows.findIndex((r) => r.getAttribute("aria-selected") === "true");
  const next = rows[Math.max(0, Math.min(rows.length - 1, at < 0 ? 0 : at + step))];
  next.scrollIntoView({ block: "nearest" });
  return next.dataset.id;
}

function sortHeader(columns, sort, onSort) {
  return h(
    "tr",
    {},
    columns.map((c) =>
      h(
        "th",
        { class: c.class, "aria-sort": sort.key === c.key ? (sort.dir > 0 ? "ascending" : "descending") : null },
        c.sort
          ? h(
              "button",
              { type: "button", onclick: () => onSort(c.key) },
              c.label,
              sort.key === c.key ? h("span", { class: "arrow" }, sort.dir > 0 ? "↑" : "↓") : null,
            )
          : c.label,
      ),
    ),
  );
}

function compare(a, b) {
  return typeof a === "number" && typeof b === "number" ? a - b : String(a).localeCompare(String(b));
}


// ---------- worklist ----------

// kept across pages, so coming back from the viewer finds the list as it was
const work = { q: "", modalities: new Set(), checked: new Set(), range: "all", from: "", to: "", sort: { key: "date", dir: -1 }, shown: 200, selected: null };

const RANGES = [
  ["today", "Today", 0],
  ["7d", "7D", 7],
  ["30d", "30D", 30],
  ["1y", "1Y", 365],
  ["all", "All", null],
];

const STUDY_COLUMNS = [
  { key: "date", label: "Date", sort: true, value: (s) => s.date + s.time, cell: (s) => h("span", { class: "mono" }, dicomDate(s.date)) },
  { key: "time", label: "Time", class: "hide-lg", cell: (s) => h("span", { class: "mono faint" }, dicomTime(s.time)) },
  { key: "name", label: "Patient", sort: true, value: (s) => personName(s.name), cell: (s) => personName(s.name), td: "pn" },
  { key: "patientId", label: "Patient ID", class: "hide-lg", sort: true, value: (s) => s.patientId, cell: (s) => h("span", { class: "mono" }, s.patientId) },
  { key: "sex", label: "Sex/Age", class: "hide-md", cell: (s) => h("span", { class: "mono" }, sexAge(s.sex, s.birth, s.date)) },
  { key: "description", label: "Description", sort: true, value: (s) => s.description, cell: (s) => s.description || h("span", { class: "faint" }, "—") },
  { key: "accession", label: "Accession", class: "hide-lg", sort: true, value: (s) => s.accession, cell: (s) => h("span", { class: "mono" }, s.accession || "—") },
  { key: "modalities", label: "Mod", sort: true, value: (s) => s.modalities.join(","), cell: (s) => s.modalities.map((m) => h("span", { class: "mod" }, m)) },
  { key: "series", label: "Ser", class: "num hide-sm", sort: true, value: (s) => s.series, cell: (s) => String(s.series) },
  { key: "instances", label: "Img", class: "num", sort: true, value: (s) => s.instances, cell: (s) => s.instances.toLocaleString() },
];

function groupLabel(da) {
  const d = parseDate(da);
  if (!d) return "No date";
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const days = Math.round((today - d) / 86400000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

async function worklistPage([selected], params) {
  let studies = await loadStudies();
  if (selected) work.selected = selected;
  let patientFilter = params.get("patient");

  const input = h("input", { type: "search", placeholder: "Name, patient ID, accession, description", "aria-label": "Search studies", value: work.q });
  const modBox = h("div", { class: "toggles", role: "group", "aria-label": "Modality" });
  const rangeBox = h("div", { class: "toggles", role: "group", "aria-label": "Study date" });
  // a range of your own: YYYY-MM-DD from the date inputs, compared with Study Date
  const dateInput = (key, label) =>
    h("input", {
      type: "date",
      class: "date",
      "aria-label": label,
      title: label,
      value: work[key],
      onchange: (e) => {
        work[key] = e.target.value;
        work.range = work.from || work.to ? "custom" : "all";
        render();
      },
    });
  const fromInput = dateInput("from", "Studies from");
  const toInput = dateInput("to", "Studies up to");
  const dates = h("span", { class: "dates" }, fromInput, h("span", { class: "faint" }, "–"), toInput);
  const patientChip = h("span");
  const count = h("span", { class: "count" });
  const head = h("thead");
  const body = h("tbody");
  const more = h("div");
  const bulk = h("div", { class: "bulk", hidden: true, role: "region", "aria-label": "Checked studies" });
  const pane = h("aside", { class: "pane", "aria-label": "Selected study" });
  const scroll = h("div", { class: "scroll" }, h("table", { class: "grid" }, head, body), more);

  view.replaceChildren(
    h(
      "div",
      { class: "work" },
      h(
        "div",
        { class: "list" },
        h("div", { class: "filters" }, h("label", { class: "field" }, searchIcon(), input, h("kbd", { class: "hide-sm" }, "/")), modBox, rangeBox, dates, patientChip, count),
        scroll,
        bulk,
      ),
      pane,
    ),
  );

  function visible() {
    const words = work.q.toLowerCase().split(/\s+/).filter(Boolean);
    const days = RANGES.find(([k]) => k === work.range)?.[2];
    let since = null;
    if (days != null) {
      since = new Date();
      since.setHours(0, 0, 0, 0);
      since.setDate(since.getDate() - days);
    }
    const col = STUDY_COLUMNS.find((c) => c.key === work.sort.key);
    const custom = work.range === "custom";
    const lo = custom && work.from ? work.from.replaceAll("-", "") : null;
    const hi = custom && work.to ? work.to.replaceAll("-", "") : null;
    return studies
      .filter((s) => (!lo || s.date >= lo) && (!hi || (s.date && s.date <= hi)))
      .filter((s) => !patientFilter || s.patient === patientFilter)
      .filter((s) => !work.modalities.size || s.modalities.some((m) => work.modalities.has(m)))
      .filter((s) => !since || (parseDate(s.date) && parseDate(s.date) >= since))
      .filter((s) => {
        if (!words.length) return true;
        const text = `${s.name} ${personName(s.name)} ${s.patientId} ${s.accession} ${s.description} ${s.modalities.join(" ")} ${s.date}`.toLowerCase();
        return words.every((w) => text.includes(w));
      })
      .sort((a, b) => work.sort.dir * compare(col.value(a), col.value(b)));
  }

  function renderFilters() {
    const all = [...new Set(studies.flatMap((s) => s.modalities))].sort();
    modBox.replaceChildren(
      ...all.map((m) =>
        h(
          "button",
          {
            class: "toggle",
            type: "button",
            "aria-pressed": String(work.modalities.has(m)),
            onclick: () => {
              work.modalities.has(m) ? work.modalities.delete(m) : work.modalities.add(m);
              render();
            },
          },
          m,
        ),
      ),
    );
    modBox.hidden = all.length < 2;
    rangeBox.replaceChildren(
      ...RANGES.map(([key, label]) =>
        h(
          "button",
          {
            class: "toggle",
            type: "button",
            "aria-pressed": String(work.range === key),
            onclick: () => {
              work.range = key;
              work.from = work.to = fromInput.value = toInput.value = "";
              render();
            },
          },
          label,
        ),
      ),
    );
    const owner = patientFilter && studies.find((s) => s.patient === patientFilter);
    patientChip.replaceChildren(
      patientFilter
        ? h(
            "span",
            { class: "chip-x" },
            `Patient ${owner ? personName(owner.name) : ""}`,
            h("button", { type: "button", "aria-label": "Show all patients", onclick: () => (location.hash = "#/studies") }, "×"),
          )
        : "",
    );
  }

  // checkboxes: several studies at once, for one ZIP or one deletion
  let lastChecked = null;
  function check(id, on, range, list) {
    const ids = [id];
    if (range && lastChecked) {
      const a = list.findIndex((s) => s.id === lastChecked), b = list.findIndex((s) => s.id === id);
      if (a >= 0 && b >= 0) ids.splice(0, 1, ...list.slice(Math.min(a, b), Math.max(a, b) + 1).map((s) => s.id));
    }
    for (const x of ids) on ? work.checked.add(x) : work.checked.delete(x);
    lastChecked = id;
    render();
  }

  function renderBulk() {
    const ids = [...work.checked].filter((id) => studies.some((s) => s.id === id));
    work.checked = new Set(ids);
    bulk.hidden = !ids.length;
    if (!ids.length) return bulk.replaceChildren();
    const chosen = studies.filter((s) => work.checked.has(s.id));
    const shown = new Set(visible().map((s) => s.id));
    const hidden = ids.filter((id) => !shown.has(id)).length;
    const images = chosen.reduce((n, s) => n + s.instances, 0);
    // a link, so the browser streams the ZIP to disk; IDs are 44 characters each
    const tooMany = ids.length > 1000;
    bulk.replaceChildren(
      h("b", {}, `${plural(ids.length, "study", "studies")} checked`),
      h("span", { class: "muted" }, `${images.toLocaleString()} images${hidden ? ` · ${hidden} hidden by the filters` : ""}`),
      h("span", { class: "spacer" }),
      h(
        "a",
        {
          class: "btn",
          href: tooMany ? null : new URL(`tools/create-archive?resources=${ids.join(",")}`, ROOT).href,
          download: "",
          title: tooMany ? "More than 1000 studies: download them in parts" : "One ZIP, a folder per patient, study and series",
        },
        "Download ZIP",
      ),
      h("button", { class: "btn danger", type: "button", onclick: () => deleteChecked(chosen) }, "Delete…"),
      h("button", { class: "btn", type: "button", title: "Uncheck all (Esc)", onclick: () => (work.checked.clear(), render()) }, "Clear"),
    );
  }

  async function deleteChecked(chosen) {
    const images = chosen.reduce((n, s) => n + s.instances, 0);
    const patients = new Set(chosen.map((s) => s.patient)).size;
    if (!(await confirmDelete(`Delete ${plural(chosen.length, "study", "studies")}?`, `${plural(chosen.length, "study", "studies")} of ${plural(patients, "patient")}, ${plural(images, "image")}. The files are removed from storage; this cannot be undone.`))) return;
    const gone = new Set();
    const failed = [];
    for (const [i, s] of chosen.entries()) {
      bulk.querySelector("b").textContent = `Deleting ${i + 1} of ${chosen.length}…`;
      try {
        await api(`studies/${s.id}`, { method: "DELETE" });
        gone.add(s.id);
      } catch (e) {
        if (e.status === 404) gone.add(s.id);
        else failed.push(`${s.description || s.id}: ${e.message}`);
      }
    }
    for (const id of gone) work.checked.delete(id);
    toast(failed.length ? `${gone.size} deleted, ${failed.length} not: ${failed[0]}` : `${plural(gone.size, "study", "studies")} deleted`, failed.length ? "error" : "");
    const wasSelected = work.selected && gone.has(work.selected);
    studies = store.studies = studies.filter((s) => !gone.has(s.id));
    render();
    if (wasSelected) history.replaceState(null, "", "#/studies");
    if (wasSelected || !work.selected) summary();
  }

  function columns(list) {
    const all = list.length > 0 && list.every((s) => work.checked.has(s.id));
    const some = list.some((s) => work.checked.has(s.id));
    const box = h("input", {
      type: "checkbox",
      "aria-label": "Check all shown studies",
      onclick: (e) => {
        e.stopPropagation();
        for (const s of list) e.target.checked ? work.checked.add(s.id) : work.checked.delete(s.id);
        render();
      },
    });
    box.checked = all;
    box.indeterminate = some && !all;
    return [
      {
        key: "check",
        label: box,
        class: "chk",
        cell: (s) => {
          const c = h("input", {
            type: "checkbox",
            "aria-label": `Check ${s.description || "study"} of ${personName(s.name)}`,
            onclick: (e) => {
              e.stopPropagation();
              check(s.id, e.target.checked, e.shiftKey, list);
            },
          });
          c.checked = work.checked.has(s.id);
          return c;
        },
      },
      ...STUDY_COLUMNS,
    ];
  }

  function render() {
    renderFilters();
    const list = visible();
    const cols = columns(list);
    head.replaceChildren(
      sortHeader(cols, work.sort, (key) => {
        work.sort = { key, dir: work.sort.key === key ? -work.sort.dir : key === "date" ? -1 : 1 };
        render();
      }),
    );
    renderBulk();
    count.textContent = list.length === studies.length ? plural(studies.length, "study", "studies") : `${list.length} of ${plural(studies.length, "study", "studies")}`;
    const rows = [];
    let group = null;
    for (const s of list.slice(0, work.shown)) {
      if (work.sort.key === "date") {
        const g = groupLabel(s.date);
        if (g !== group) rows.push(h("tr", { class: "group" }, h("td", { colspan: cols.length }, (group = g))));
      }
      rows.push(
        h(
          "tr",
          {
            "data-id": s.id,
            "aria-selected": String(s.id === work.selected),
            class: work.checked.has(s.id) ? "checked" : null,
            onclick: (e) => (e.target.closest(".chk") ? null : select(s.id)),
            ondblclick: () => openViewer(s.id),
          },
          cols.map((c) => h("td", { class: [c.class, c.td].filter(Boolean).join(" ") || null }, c.cell(s))),
        ),
      );
    }
    body.replaceChildren(...rows);
    if (!studies.length) body.replaceChildren(h("tr", {}, h("td", { colspan: cols.length }, emptyArchive())));
    else if (!list.length) body.replaceChildren(h("tr", {}, h("td", { colspan: cols.length, class: "nothing" }, h("b", {}, "No study matches"), "Change the search or the filters.")));
    more.replaceChildren(
      list.length > work.shown
        ? h("button", { class: "more", type: "button", onclick: () => ((work.shown += 200), render()) }, `Show more (${(list.length - work.shown).toLocaleString()} left)`)
        : "",
    );
    const patients = new Set(list.map((s) => s.patient)).size;
    setStatus(
      [h("b", {}, plural(list.length, "study", "studies")), plural(patients, "patient"), `${list.reduce((n, s) => n + s.instances, 0).toLocaleString()} images`],
      [["↑↓", "select"], ["↵", "open"], ["x", "check"], ["/", "search"], ["Del", "delete"]],
    );
  }

  let token = 0;
  async function select(id, { replace = true } = {}) {
    work.selected = id;
    for (const tr of body.querySelectorAll("tr[data-id]")) tr.setAttribute("aria-selected", String(tr.dataset.id === id));
    const hash = `#/studies/${id}${patientFilter ? `?patient=${patientFilter}` : ""}`;
    if (replace && location.hash !== hash) history.replaceState(null, "", hash);
    const mine = ++token;
    const study = studies.find((s) => s.id === id);
    if (!study) return summary();
    pane.classList.remove("empty");
    pane.replaceChildren(...studyPane(study, null, null));
    try {
      const [series, stats] = await Promise.all([api(`studies/${id}/series`), api(`studies/${id}/statistics`)]);
      if (mine !== token) return;
      pane.replaceChildren(...studyPane(study, series, stats));
    } catch (e) {
      if (mine === token) pane.replaceChildren(failure(e));
    }
  }

  function summary() {
    work.selected = null;
    for (const tr of body.querySelectorAll('tr[aria-selected="true"]')) tr.setAttribute("aria-selected", "false");
    pane.classList.add("empty");
    const patients = new Set(studies.map((s) => s.patient)).size;
    pane.replaceChildren(
      h(
        "section",
        {},
        h("div", { class: "label", style: "margin-bottom: 10px" }, "Archive"),
        h(
          "div",
          { class: "stats" },
          h("div", {}, h("b", {}, studies.length.toLocaleString()), h("span", { class: "label" }, "Studies")),
          h("div", {}, h("b", {}, patients.toLocaleString()), h("span", { class: "label" }, "Patients")),
          h("div", {}, h("b", {}, studies.reduce((n, s) => n + s.instances, 0).toLocaleString()), h("span", { class: "label" }, "Images")),
        ),
      ),
      h(
        "section",
        {},
        h("div", { class: "label", style: "margin-bottom: 6px" }, "Keys"),
        h(
          "div",
          { class: "hint" },
          [["/", "search"], ["↑ ↓", "move through the list"], ["↵", "open the selected study"], ["x", "check it; shift-click checks a range"], ["Del", "delete the selected or checked"], ["Esc", "clear the search or the checks"]].map(([k, w]) =>
            h("div", {}, h("kbd", {}, k), w),
          ),
        ),
      ),
      h("section", {}, h("div", { class: "hint" }, "Drop DICOM files or folders anywhere to store them.")),
    );
  }

  function studyPane(s, series, stats) {
    const others = studies.filter((o) => o.patient === s.patient && o.id !== s.id).sort((a, b) => compare(b.date, a.date));
    const waiting = h("span", { class: "wait" }, "…");
    return [
      h("button", { class: "pane-close", type: "button", onclick: () => (history.replaceState(null, "", "#/studies"), summary()) }, "← Back to the list"),
      h(
        "section",
        {},
        h(
          "div",
          { class: "who" },
          h("a", { class: "pn", href: `#/studies?patient=${s.patient}`, title: "Only this patient's studies" }, personName(s.name)),
          h("span", { class: "mono muted" }, sexAge(s.sex, s.birth, s.date)),
        ),
        h("div", { class: "sub" }, `${s.patientId || "no ID"} · born ${dicomDate(s.birth)}`),
      ),
      h(
        "section",
        {},
        h("h2", {}, s.description || "No description"),
        h("div", { class: "sub" }, `${longDate(s.date)}${s.time ? ` · ${dicomTime(s.time)}` : ""}`, "  ", s.modalities.map((m) => h("span", { class: "mod" }, m))),
        h(
          "div",
          { class: "actions" },
          h("a", { class: "btn primary", href: series?.length ? `#/view/${s.id}/${sortSeries(series)[0].ID}` : null, onclick: series ? null : (e) => e.preventDefault() }, "Open ", h("kbd", {}, "↵")),
          h("a", { class: "btn", href: new URL(`studies/${s.id}/archive`, ROOT).href, download: "" }, "Download ZIP"),
          h("span", { class: "spacer" }),
          h("button", { class: "btn danger", type: "button", onclick: () => deleteStudy(s, stats) }, "Delete"),
        ),
      ),
      h(
        "section",
        {},
        h(
          "div",
          { class: "stats" },
          h("div", {}, h("b", {}, String(s.series)), h("span", { class: "label" }, "Series")),
          h("div", {}, h("b", {}, s.instances.toLocaleString()), h("span", { class: "label" }, "Images")),
          h("div", {}, h("b", {}, stats ? bytes(stats.DiskSize) : waiting), h("span", { class: "label" }, "Stored")),
        ),
      ),
      h(
        "section",
        {},
        h("div", { class: "section-head" }, h("span", { class: "label" }, "Series"), h("span", { class: "wait" }, "click to view")),
        series ? h("div", { class: "thumbs" }, sortSeries(series).map((x) => seriesThumb(x, s))) : h("div", { class: "bar-load" }),
      ),
      h(
        "section",
        {},
        h("div", { class: "label", style: "margin-bottom: 6px" }, "Details"),
        h(
          "dl",
          { class: "kv" },
          kv("Accession", h("span", { class: "mono" }, s.accession || "—")),
          kv("Study ID", h("span", { class: "mono" }, s.studyId || "—")),
          kv("Referring", personName(s.referring)),
          kv("Patient ID", copyable(s.patientId)),
          kv("Study UID", copyable(s.uid)),
          kv("Uncompressed", stats ? bytes(stats.UncompressedSize) : "…"),
          kv("Last update", h("span", { class: "mono" }, s.updated.replace(/^(\d{4})(\d\d)(\d\d)T(\d\d)(\d\d).*/, "$1-$2-$3 $4:$5"))),
        ),
      ),
      h(
        "section",
        {},
        h("div", { class: "section-head" }, h("span", { class: "label" }, others.length ? `Other studies of this patient · ${others.length}` : "Patient")),
        others.length
          ? h(
              "div",
              { class: "others" },
              others.slice(0, 12).map((o) =>
                h(
                  "a",
                  { href: `#/studies/${o.id}`, onclick: (e) => (e.preventDefault(), select(o.id)) },
                  h("span", { class: "mono muted" }, dicomDate(o.date)),
                  o.modalities.map((m) => h("span", { class: "mod" }, m)),
                  h("span", {}, o.description || "—"),
                ),
              ),
            )
          : null,
        h(
          "div",
          { class: "actions" },
          h("a", { class: "btn", href: new URL(`patients/${s.patient}/archive`, ROOT).href, download: "" }, "Download patient"),
          h("span", { class: "spacer" }),
          h("button", { class: "btn danger", type: "button", onclick: () => deletePatient(s) }, "Delete patient"),
        ),
      ),
    ];
  }

  async function deleteStudy(s, stats) {
    const what = stats ? `${plural(stats.CountSeries, "series", "series")}, ${plural(stats.CountInstances, "image")}` : plural(s.instances, "image");
    if (!(await confirmDelete("Delete this study?", `${s.description || "The study"} of ${personName(s.name)}, ${dicomDate(s.date)}: ${what}. The files are removed from storage; this cannot be undone.`))) return;
    try {
      await api(`studies/${s.id}`, { method: "DELETE" });
      toast("Study deleted");
      afterDelete((x) => x.id !== s.id);
    } catch (e) {
      toast(`Could not delete: ${e.message}`, "error");
    }
  }

  async function deletePatient(s) {
    const theirs = studies.filter((x) => x.patient === s.patient);
    if (!(await confirmDelete("Delete this patient?", `${personName(s.name)} (${s.patientId || "no ID"}) and all of their ${plural(theirs.length, "study", "studies")}: ${plural(theirs.reduce((n, x) => n + x.instances, 0), "image")}. This cannot be undone.`))) return;
    try {
      await api(`patients/${s.patient}`, { method: "DELETE" });
      toast("Patient deleted");
      if (patientFilter === s.patient) patientFilter = null;
      afterDelete((x) => x.patient !== s.patient);
    } catch (e) {
      toast(`Could not delete: ${e.message}`, "error");
    }
  }

  function afterDelete(keep) {
    const list = visible();
    const at = list.findIndex((x) => x.id === work.selected);
    studies = store.studies = studies.filter(keep);
    const after = visible();
    const next = after[Math.min(at, after.length - 1)];
    render();
    if (next) select(next.id);
    else {
      history.replaceState(null, "", "#/studies");
      summary();
    }
  }

  function openViewer(id) {
    api(`studies/${id}/series`).then((series) => {
      if (series.length) location.hash = `#/view/${id}/${sortSeries(series)[0].ID}`;
    });
  }

  input.addEventListener("input", () => {
    work.q = input.value;
    work.shown = 200;
    render();
  });

  const keys = (e) => {
    if (e.key === "/" && !isTyping(e)) {
      e.preventDefault();
      input.focus();
      input.select();
      return;
    }
    if (e.key === "Escape" && document.activeElement === input) {
      input.value = work.q = "";
      render();
      input.blur();
      return;
    }
    if (isTyping(e) && !(document.activeElement === input && ["ArrowDown", "ArrowUp", "Enter"].includes(e.key))) return;
    if (e.key === "x" && work.selected) {
      check(work.selected, !work.checked.has(work.selected), false, visible());
    } else if (e.key === "Escape" && work.checked.size) {
      work.checked.clear();
      render();
    } else if (e.key === "Escape" && work.selected && matchMedia("(max-width: 980px)").matches) {
      history.replaceState(null, "", "#/studies");
      summary();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const id = moveSelection(body, e.key === "ArrowDown" ? 1 : -1);
      if (id) select(id);
    } else if (e.key === "Enter" && work.selected) {
      e.preventDefault();
      openViewer(work.selected);
    } else if (e.key === "Delete" && work.checked.size) {
      deleteChecked(studies.filter((x) => work.checked.has(x.id)));
    } else if (e.key === "Delete" && work.selected) {
      const s = studies.find((x) => x.id === work.selected);
      if (s) deleteStudy(s, null);
    }
  };
  document.addEventListener("keydown", keys);

  render();
  if (work.selected && studies.some((s) => s.id === work.selected)) {
    select(work.selected);
    requestAnimationFrame(() => body.querySelector('tr[aria-selected="true"]')?.scrollIntoView({ block: "center" }));
  } else summary();
  if (!work.selected) input.focus();

  return {
    cleanup: () => document.removeEventListener("keydown", keys),
    update: ([id], p) => {
      if ((p.get("patient") || null) !== patientFilter) {
        patientFilter = p.get("patient");
        render();
      }
      if (id && id !== work.selected) select(id, { replace: false });
      else if (!id && work.selected && !patientFilter) render();
    },
    refresh: async () => {
      invalidate();
      studies = await loadStudies();
      render();
      if (work.selected) select(work.selected);
      else summary();
    },
  };
}

// ---------- patients ----------

const people = { q: "", sort: { key: "name", dir: 1 } };

async function patientsPage() {
  const studies = await loadStudies();
  const byPatient = new Map();
  for (const s of studies) {
    const p = byPatient.get(s.patient) || { id: s.patient, name: s.name, patientId: s.patientId, sex: s.sex, birth: s.birth, studies: 0, last: "", modalities: new Set(), instances: 0 };
    p.studies++;
    p.instances += s.instances;
    if (s.date > p.last) p.last = s.date;
    s.modalities.forEach((m) => p.modalities.add(m));
    byPatient.set(s.patient, p);
  }
  const patients = [...byPatient.values()];
  const columns = [
    { key: "name", label: "Patient", sort: true, value: (p) => personName(p.name), cell: (p) => personName(p.name), td: "pn" },
    { key: "patientId", label: "Patient ID", sort: true, value: (p) => p.patientId, cell: (p) => h("span", { class: "mono" }, p.patientId || "—") },
    { key: "sex", label: "Sex/Age", class: "hide-sm", sort: true, value: (p) => p.sex, cell: (p) => h("span", { class: "mono" }, sexAge(p.sex, p.birth)) },
    { key: "birth", label: "Born", class: "hide-md", sort: true, value: (p) => p.birth, cell: (p) => h("span", { class: "mono" }, dicomDate(p.birth)) },
    { key: "modalities", label: "Mod", class: "hide-sm", cell: (p) => [...p.modalities].sort().map((m) => h("span", { class: "mod" }, m)) },
    { key: "last", label: "Last study", sort: true, value: (p) => p.last, cell: (p) => h("span", { class: "mono" }, dicomDate(p.last)) },
    { key: "studies", label: "Studies", class: "num", sort: true, value: (p) => p.studies, cell: (p) => String(p.studies) },
    { key: "instances", label: "Img", class: "num hide-sm", sort: true, value: (p) => p.instances, cell: (p) => p.instances.toLocaleString() },
  ];
  const input = h("input", { type: "search", placeholder: "Name or patient ID", "aria-label": "Search patients", value: people.q });
  const count = h("span", { class: "count" });
  const head = h("thead");
  const body = h("tbody");
  let selected = null;
  view.replaceChildren(
    h(
      "div",
      { class: "list", style: "height: 100%" },
      h("div", { class: "filters" }, h("label", { class: "field" }, searchIcon(), input, h("kbd", { class: "hide-sm" }, "/")), count),
      h("div", { class: "scroll" }, h("table", { class: "grid" }, head, body)),
    ),
  );
  const open = (id) => (location.hash = `#/studies?patient=${id}`);
  function render() {
    head.replaceChildren(
      sortHeader(columns, people.sort, (key) => {
        people.sort = { key, dir: people.sort.key === key ? -people.sort.dir : ["last", "studies", "instances"].includes(key) ? -1 : 1 };
        render();
      }),
    );
    const words = people.q.toLowerCase().split(/\s+/).filter(Boolean);
    const col = columns.find((c) => c.key === people.sort.key);
    const list = patients
      .filter((p) => words.every((w) => `${p.name} ${personName(p.name)} ${p.patientId}`.toLowerCase().includes(w)))
      .sort((a, b) => people.sort.dir * compare(col.value(a), col.value(b)));
    count.textContent = list.length === patients.length ? plural(patients.length, "patient") : `${list.length} of ${plural(patients.length, "patient")}`;
    body.replaceChildren(
      ...list.slice(0, 1000).map((p) =>
        h(
          "tr",
          { "data-id": p.id, "aria-selected": String(p.id === selected), onclick: () => select(p.id), ondblclick: () => open(p.id) },
          columns.map((c) => h("td", { class: [c.class, c.td].filter(Boolean).join(" ") || null }, c.cell(p))),
        ),
      ),
    );
    if (!patients.length) body.replaceChildren(h("tr", {}, h("td", { colspan: columns.length }, emptyArchive())));
    else if (!list.length) body.replaceChildren(h("tr", {}, h("td", { colspan: columns.length, class: "nothing" }, h("b", {}, "No patient matches"))));
    setStatus([h("b", {}, plural(list.length, "patient")), plural(list.reduce((n, p) => n + p.studies, 0), "study", "studies")], [["↑↓", "select"], ["↵", "their studies"], ["/", "search"]]);
  }
  function select(id) {
    selected = id;
    for (const tr of body.querySelectorAll("tr[data-id]")) tr.setAttribute("aria-selected", String(tr.dataset.id === id));
  }
  input.addEventListener("input", () => ((people.q = input.value), render()));
  const keys = (e) => {
    if (e.key === "/" && !isTyping(e)) {
      e.preventDefault();
      input.focus();
      return;
    }
    if (e.key === "Escape" && document.activeElement === input) {
      input.value = people.q = "";
      render();
      input.blur();
      return;
    }
    if (isTyping(e) && !(document.activeElement === input && ["ArrowDown", "ArrowUp", "Enter"].includes(e.key))) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const id = moveSelection(body, e.key === "ArrowDown" ? 1 : -1);
      if (id) select(id);
    } else if (e.key === "Enter" && selected) open(selected);
  };
  document.addEventListener("keydown", keys);
  render();
  input.focus();
  return {
    cleanup: () => document.removeEventListener("keydown", keys),
    refresh: async () => {
      invalidate();
      page?.cleanup?.();
      page = null;
      route();
    },
  };
}

// ---------- upload: drop anywhere, progress in the tray ----------

const tray = document.getElementById("tray");
const dropzone = document.getElementById("dropzone");
const uploadButton = document.getElementById("upload-button");
const uploadMenu = document.getElementById("upload-menu");
const pickFiles = document.getElementById("pick-files");
const pickFolder = document.getElementById("pick-folder");
let upload = null;

function pick(kind) {
  closeMenu();
  (kind === "folder" ? pickFolder : pickFiles).click();
}
function closeMenu() {
  uploadMenu.hidden = true;
  uploadButton.setAttribute("aria-expanded", "false");
}
uploadButton.addEventListener("click", (e) => {
  e.stopPropagation();
  uploadMenu.hidden = !uploadMenu.hidden;
  uploadButton.setAttribute("aria-expanded", String(!uploadMenu.hidden));
  if (!uploadMenu.hidden) uploadMenu.querySelector("button").focus();
});
uploadMenu.addEventListener("click", (e) => {
  const kind = e.target.closest("[data-pick]")?.dataset.pick;
  if (kind) pick(kind);
});
document.addEventListener("click", (e) => {
  if (!uploadMenu.hidden && !uploadMenu.contains(e.target)) closeMenu();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !uploadMenu.hidden) {
    closeMenu();
    uploadButton.focus();
  }
});
for (const input of [pickFiles, pickFolder])
  input.addEventListener("change", () => {
    storeFiles([...input.files]);
    input.value = "";
  });

// the overlay shows while files are dragged over the window
let dragDepth = 0;
const dragsFiles = (e) => [...(e.dataTransfer?.types || [])].includes("Files");
addEventListener("dragenter", (e) => {
  if (!dragsFiles(e)) return;
  e.preventDefault();
  dragDepth++;
  dropzone.hidden = false;
});
addEventListener("dragover", (e) => {
  if (dragsFiles(e)) e.preventDefault();
});
addEventListener("dragleave", () => {
  if (--dragDepth <= 0) {
    dragDepth = 0;
    dropzone.hidden = true;
  }
});
addEventListener("drop", async (e) => {
  if (!dragsFiles(e)) return;
  e.preventDefault();
  dragDepth = 0;
  dropzone.hidden = true;
  storeFiles(await droppedFiles(e.dataTransfer));
});

function storeFiles(files) {
  files = files.filter((f) => f.size > 0 && !/(^|\/)(DICOMDIR|\.DS_Store|Thumbs\.db|desktop\.ini)$/i.test(f.webkitRelativePath || f.name));
  if (!files.length) return toast("No files to store");
  const size = files.reduce((n, f) => n + f.size, 0);
  if (upload?.running) {
    upload.queue.push(...files);
    upload.total += files.length;
    upload.totalBytes += size;
  } else {
    upload = { queue: [...files], total: files.length, totalBytes: size, sentBytes: 0, stored: 0, already: 0, failed: [], studies: new Set(), lastStudy: null, running: true, started: performance.now() };
    runUpload(upload);
  }
  paintTray();
}

async function runUpload(u) {
  let last = 0;
  const worker = async () => {
    while (u.queue.length) {
      const file = u.queue.shift();
      try {
        const result = await api("instances", { method: "POST", body: file, headers: { "Content-Type": "application/dicom" } });
        if (result.Status === "AlreadyStored") u.already++;
        else u.stored++;
        u.studies.add(result.ParentStudy);
        u.lastStudy = result.ParentStudy;
      } catch (e) {
        u.failed.push([file.webkitRelativePath || file.name, e.status === 400 ? "not DICOM" : e.message]);
      }
      u.sentBytes += file.size;
      if (performance.now() - last > 120) {
        last = performance.now();
        paintTray();
      }
    }
  };
  await Promise.all(Array.from({ length: 4 }, worker));
  u.running = false;
  u.seconds = (performance.now() - u.started) / 1000;
  paintTray();
  if (u.stored) await page?.refresh?.();
}

function paintTray() {
  const u = upload;
  if (!u) return (tray.hidden = true);
  tray.hidden = false;
  const done = u.stored + u.already + u.failed.length;
  const pct = u.totalBytes ? Math.round((100 * u.sentBytes) / u.totalBytes) : 100;
  tray.replaceChildren(
    h(
      "header",
      {},
      h("span", { class: "label" }, u.running ? `Storing ${done.toLocaleString()} / ${u.total.toLocaleString()}` : "Stored"),
      h("span", { class: "mono muted" }, u.running ? `${pct}%` : `${u.seconds.toFixed(1)} s`),
      u.running ? null : h("button", { class: "x", type: "button", "aria-label": "Close", onclick: () => ((upload = null), paintTray()) }, "×"),
    ),
    h(
      "div",
      { class: "body" },
      h("div", { class: "meter", role: "progressbar", "aria-valuenow": String(pct), "aria-valuemin": "0", "aria-valuemax": "100" }, h("i", { style: `width: ${pct}%` })),
      h("div", { class: "row" }, "new", h("b", {}, u.stored.toLocaleString())),
      h("div", { class: "row" }, "already stored", h("b", {}, u.already.toLocaleString())),
      h("div", { class: "row" }, "skipped", h("b", { class: u.failed.length ? "bad" : null }, u.failed.length.toLocaleString())),
      h("div", { class: "row" }, "sent", h("b", {}, `${bytes(u.sentBytes)} of ${bytes(u.totalBytes)}`)),
      u.failed.length
        ? h("details", {}, h("summary", {}, "skipped files"), h("ul", {}, u.failed.slice(0, 300).map(([name, why]) => h("li", {}, h("span", { title: name }, name), h("span", {}, why)))))
        : null,
      !u.running && u.lastStudy
        ? h(
            "div",
            {},
            h(
              "a",
              {
                class: "btn primary",
                href: `#/studies/${u.lastStudy}`,
                onclick: () => {
                  work.q = "";
                  work.modalities.clear();
                  work.range = "all";
                },
              },
              u.studies.size === 1 ? "Show the study" : `Show the last of ${u.studies.size} studies`,
            ),
          )
        : null,
    ),
  );
}
addEventListener("beforeunload", (e) => {
  if (upload?.running) e.preventDefault();
});

// files from a drop, folders walked
async function droppedFiles(transfer) {
  const entries = [...transfer.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
  if (!entries.length) return [...transfer.files];
  const files = [];
  async function walk(entry, path) {
    if (entry.isFile) {
      const file = await new Promise((ok, fail) => entry.file(ok, fail));
      Object.defineProperty(file, "webkitRelativePath", { value: path + file.name });
      files.push(file);
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      for (;;) {
        const batch = await new Promise((ok, fail) => reader.readEntries(ok, fail));
        if (!batch.length) break;
        for (const child of batch) await walk(child, `${path}${entry.name}/`);
      }
    }
  }
  for (const entry of entries) await walk(entry, "");
  return files;
}


route();

// the viewer and Cornerstone3D load only when a series is opened
async function viewerPage(args) {
  const viewer = await import("./viewer.js");
  return viewer.viewerPage(args, view);
}
