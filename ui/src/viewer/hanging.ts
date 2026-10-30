// Hanging protocols: which layout a study opens in, and which series goes in each viewport.
// A protocol describes series by kind (the current study or the latest prior, a modality, words
// of the description, the largest or the n-th), not by UID, so one protocol serves every study
// it matches. The built-in ones are here; the users' own are kept on the server
// (/ui/api/protocols), shared by every browser, and come first when they match.

import { api } from "@/lib/api";
import { sortSeries } from "@/lib/studies";
import { LAYOUTS } from "@/viewer/controller";

export type SeriesRule = {
  from: "current" | "prior";
  /** only series of this modality */
  modality?: string;
  /** words the series description should share */
  description?: string;
  /** which of the candidates: the most images, the n-th in series order, or the one most like
   * the series shown in viewport `like` (for priors) */
  pick?: "largest" | { nth: number } | { like: number };
};

export type Protocol = {
  id: string;
  name: string;
  builtin?: boolean;
  /** applies to studies with one of these modalities; empty: any */
  modalities: string[];
  /** a LAYOUTS index, or "auto" for one viewport per series, up to 2 × 3 */
  layout: number | "auto";
  viewports: SeriesRule[];
  linked?: boolean;
  /** open as MPR on the first viewport's series */
  mpr?: boolean;
  priority: number;
};

export type Plan = { protocol: Protocol; layout: number; series: (string | null)[]; linked: boolean; mpr: boolean };

const MAX_AUTO = 6;

export const BUILTIN: Protocol[] = [
  {
    id: "builtin:prior",
    name: "Current and prior",
    builtin: true,
    modalities: [],
    layout: 1,
    viewports: [
      { from: "current", pick: "largest" },
      { from: "prior", pick: { like: 0 } },
    ],
    linked: true,
    priority: 5,
  },
  {
    id: "builtin:two-views",
    name: "Two views",
    builtin: true,
    modalities: ["CR", "DX", "MG", "RF"],
    layout: 1,
    viewports: [
      { from: "current", pick: { nth: 0 } },
      { from: "current", pick: { nth: 1 } },
    ],
    priority: 4,
  },
  { id: "builtin:single", name: "Largest series", builtin: true, modalities: [], layout: 0, viewports: [{ from: "current", pick: "largest" }], priority: 1 },
  {
    id: "builtin:all",
    name: "All series",
    builtin: true,
    modalities: [],
    layout: "auto",
    viewports: Array.from({ length: MAX_AUTO }, (_, i) => ({ from: "current" as const, pick: { nth: i } })),
    priority: 0,
  },
  { id: "builtin:mpr", name: "MPR of the largest series", builtin: true, modalities: ["CT", "MR", "PT", "NM"], layout: 0, viewports: [{ from: "current", pick: "largest" }], mpr: true, priority: 0 },
];

// ---- the users' own, on the server ----

/** The page is /ui/; the protocols are /ui/api/protocols. */
const PROTOCOLS = new URL("api/protocols", location.href);
/** Where this browser kept its own before the server did; moved up once. */
const OLD_KEY = "pacs-rs-protocols";
const AUTO_KEY = "pacs-rs-protocols-auto";

async function send(method: string, id: string, body?: Protocol) {
  const response = await fetch(new URL(`api/protocols/${encodeURIComponent(id)}`, location.href), {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok && response.status !== 404) {
    let message = `${response.status} ${response.statusText}`;
    try {
      message = (await response.json()).Message || message;
    } catch {}
    throw new Error(message);
  }
}
export const saveProtocol = (p: Protocol) => send("PUT", p.id, p);
export const deleteProtocol = (id: string) => send("DELETE", id);

/** The users' protocols, most recently saved first; any this browser still keeps is moved up. */
export async function fetchUserProtocols(): Promise<Protocol[]> {
  try {
    const old = JSON.parse(localStorage.getItem(OLD_KEY) || "[]");
    if (Array.isArray(old) && old.length) {
      for (const p of old) await saveProtocol(p);
      localStorage.removeItem(OLD_KEY);
    }
  } catch {}
  const response = await fetch(PROTOCOLS, { cache: "no-store" });
  if (!response.ok) return [];
  const list = await response.json();
  return Array.isArray(list) ? list.filter((p) => p && typeof p.id === "string" && Array.isArray(p.viewports)) : [];
}
export function autoApply() {
  try {
    return localStorage.getItem(AUTO_KEY) !== "off";
  } catch {
    return true;
  }
}
export function setAutoApply(on: boolean) {
  try {
    localStorage.setItem(AUTO_KEY, on ? "on" : "off");
  } catch {}
}



// ---- what a study offers ----

export type StudyContext = { study: any; series: any[]; prior: { study: any; series: any[] } | null; modalities: string[] };

const contexts = new Map<string, Promise<StudyContext>>();

/** The study's series, and its latest prior (an older study of the same patient) with its own. */
export function studyContext(studyId: string) {
  if (!contexts.has(studyId))
    contexts.set(
      studyId,
      (async () => {
        const study = await api(`studies/${studyId}`);
        const [series, studies] = await Promise.all([api<any[]>(`studies/${studyId}/series`).then(sortSeries), api<any[]>(`patients/${study.ParentPatient}/studies`)]);
        const date = String(study.MainDicomTags.StudyDate || "");
        const older = studies
          .filter((s) => s.ID !== studyId && String(s.MainDicomTags.StudyDate || "") < date)
          .sort((a, b) => String(b.MainDicomTags.StudyDate).localeCompare(String(a.MainDicomTags.StudyDate)));
        const prior = older[0] ? { study: older[0], series: sortSeries(await api<any[]>(`studies/${older[0].ID}/series`)) } : null;
        const modalities = [...new Set(series.map((s) => s.MainDicomTags.Modality).filter(Boolean))] as string[];
        return { study, series, prior, modalities };
      })().catch((e) => {
        contexts.delete(studyId);
        throw e;
      }),
    );
  return contexts.get(studyId)!;
}

const words = (s?: string) => new Set(String(s || "").toLowerCase().split(/[^a-z0-9]+/).filter((w) => w.length > 1));
function likeness(a: any, b: any) {
  const wa = words(a.MainDicomTags.SeriesDescription), wb = words(b.MainDicomTags.SeriesDescription);
  const shared = [...wa].filter((w) => wb.has(w)).length;
  const sameModality = a.MainDicomTags.Modality === b.MainDicomTags.Modality ? 2 : 0;
  const na = a.Instances.length, nb = b.Instances.length;
  const sameSize = Math.min(na, nb) / Math.max(na, nb, 1);
  return sameModality + shared / Math.max(1, Math.max(wa.size, wb.size)) + sameSize * 0.5;
}

function choose(rule: SeriesRule, ctx: StudyContext, chosen: (any | null)[]) {
  const source = rule.from === "prior" ? ctx.prior?.series : ctx.series;
  if (!source?.length) return null;
  let candidates = source.filter((s) => !rule.modality || s.MainDicomTags.Modality === rule.modality);
  if (rule.description) {
    const want = words(rule.description);
    const matching = candidates.filter((s) => [...want].every((w) => words(s.MainDicomTags.SeriesDescription).has(w)));
    if (matching.length) candidates = matching;
    else if (rule.from === "current") return null;
  }
  if (!candidates.length) return null;
  const pick = rule.pick ?? "largest";
  if (pick === "largest") return candidates.reduce((a, b) => (b.Instances.length > a.Instances.length ? b : a));
  if ("nth" in pick) return candidates[pick.nth] ?? null;
  const ref = chosen[pick.like];
  if (!ref) return candidates.reduce((a, b) => (b.Instances.length > a.Instances.length ? b : a));
  return candidates.reduce((a, b) => (likeness(b, ref) > likeness(a, ref) ? b : a));
}

/** What `protocol` puts where in this study, or null when it doesn't fit it. */
export function plan(protocol: Protocol, ctx: StudyContext): Plan | null {
  if (protocol.modalities.length && !protocol.modalities.some((m) => ctx.modalities.includes(m))) return null;
  if (protocol.viewports.some((v) => v.from === "prior") && !ctx.prior) return null;
  const chosen: (any | null)[] = [];
  for (const rule of protocol.viewports) chosen.push(choose(rule, ctx, chosen));
  let layout: number;
  if (protocol.layout === "auto") {
    const n = chosen.filter(Boolean).length;
    layout = n <= 1 ? 0 : n === 2 ? 1 : n <= 4 ? 3 : 5;
  } else layout = protocol.layout;
  const { cols, rows } = LAYOUTS[layout];
  const series = chosen.slice(0, cols * rows).map((s) => s?.ID ?? null);
  while (series.length < cols * rows) series.push(null);
  // a protocol that fills under half of its viewports doesn't fit this study
  const filled = series.filter(Boolean).length;
  if (!filled || (protocol.layout !== "auto" && filled < Math.ceil(series.length / 2))) return null;
  // two-view protocols need two different series
  if (new Set(series.filter(Boolean)).size < Math.min(filled, 2) && protocol.viewports.length > 1) return null;
  return { protocol, layout, series, linked: !!protocol.linked, mpr: !!protocol.mpr };
}

/** The protocols that fit the study, best first: the users' own, then the built-in ones. */
export function plans(ctx: StudyContext, mine: Protocol[]) {
  return [...mine, ...BUILTIN]
    .map((p) => plan(p, ctx))
    .filter((p): p is Plan => !!p)
    .sort((a, b) => (b.protocol.builtin ? 0 : 1) - (a.protocol.builtin ? 0 : 1) || b.protocol.priority - a.protocol.priority);
}

/** A protocol from what the viewports show now, to keep as the user's own. */
export function fromCurrent(name: string, layout: number, shown: { seriesId: string | null; studyId: string | null }[], ctx: StudyContext, linked: boolean, modalities: string[]): Protocol {
  const rules: SeriesRule[] = shown.map(({ seriesId, studyId }, i) => {
    const fromPrior = !!ctx.prior && studyId === ctx.prior.study.ID;
    const list = fromPrior ? ctx.prior!.series : ctx.series;
    const s = list.find((x) => x.ID === seriesId);
    if (!s) return { from: "current", pick: { nth: i } };
    const largest = list.reduce((a, b) => (b.Instances.length > a.Instances.length ? b : a));
    return {
      from: fromPrior ? "prior" : "current",
      modality: s.MainDicomTags.Modality || undefined,
      description: s === largest ? undefined : s.MainDicomTags.SeriesDescription || undefined,
      pick: fromPrior ? { like: 0 } : s === largest ? "largest" : { nth: 0 },
    };
  });
  return { id: `user:${Date.now().toString(36)}`, name, modalities, layout, viewports: rules, linked, priority: 10 };
}
