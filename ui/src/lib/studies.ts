// The archive's studies as the list shows them, a page at a time from /ui/api/studies: the
// server filters, sorts, counts and sums them up; the browser never holds the whole archive.

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";

export type Study = {
  id: string;
  patient: string;
  uid: string;
  date: string;
  time: string;
  description: string;
  accession: string;
  studyId: string;
  referring: string;
  name: string;
  patientId: string;
  birth: string;
  sex: string;
  modalities: string[];
  series: number;
  instances: number;
  /** when its latest image arrived, epoch ms, as text */
  updated: string;
};

/** What the list shows: the filters, the order, the page. */
export type StudyQuery = {
  q: string;
  from: string | null;
  to: string | null;
  modalities: string[];
  sexes: string[];
  patient: string | null;
  sort: "patient" | "description" | "modality" | "date" | "series" | "images" | "received";
  desc: boolean;
  offset: number;
  limit: number;
};
export type StudyFilters = Omit<StudyQuery, "sort" | "desc" | "offset" | "limit">;

export type StudySummary = {
  patients: number;
  studies: number;
  series: number;
  instances: number;
  modalities: [string, number][];
  recent: Study[];
};

export type StudyFacets = { modalities: Record<string, number>; sexes: Record<string, number>; dates: Record<string, number> };

/** A study as `/ui/api/studies` gives it (or the REST API, with the counts added). */
function fromList(s: any): Study {
  const t = s.MainDicomTags ?? {}, p = s.PatientMainDicomTags ?? {};
  return {
    id: s.ID,
    patient: s.ParentPatient,
    uid: t.StudyInstanceUID || "",
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
    modalities: s.Modalities ?? [],
    series: Number(s.Series ?? 0),
    instances: Number(s.Instances ?? 0),
    updated: s.LastReceived ? String(s.LastReceived) : "",
  };
}

function search(query: Partial<StudyQuery>, extra: Record<string, string> = {}) {
  const url = new URL("api/studies", location.href);
  url.hash = "";
  const set = (k: string, v: string | null | undefined) => v && url.searchParams.set(k, v);
  set("q", query.q?.trim());
  set("from", query.from);
  set("to", query.to);
  set("modality", query.modalities?.join(","));
  set("sex", query.sexes?.join(","));
  set("patient", query.patient);
  set("sort", query.sort);
  if (query.desc !== undefined) set("order", query.desc ? "desc" : "asc");
  if (query.offset) set("offset", String(query.offset));
  if (query.limit) set("limit", String(query.limit));
  for (const [k, v] of Object.entries(extra)) set(k, v);
  return url;
}

async function get(url: URL) {
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      message = (await response.json()).Message || message;
    } catch {}
    throw new Error(message);
  }
  return response.json();
}

/** One page of the list, and how many studies the whole list has; the last page stays on
 * screen while the next one loads. */
export function useStudyPage(query: StudyQuery) {
  return useQuery({
    queryKey: ["studies", "page", query],
    queryFn: async () => {
      const body = await get(search(query));
      return { total: Number(body.Total), studies: (body.Studies as any[]).map(fromList) };
    },
    placeholderData: keepPreviousData,
  });
}

/** What the whole filtered list holds (its order and pages aside). */
export function useStudySummary(filters: StudyFilters) {
  return useQuery({
    queryKey: ["studies", "summary", filters],
    queryFn: async (): Promise<StudySummary> => {
      const s = (await get(search({ ...filters, limit: 1 }, { summary: "true" }))).Summary;
      return {
        patients: s.Patients,
        studies: s.Studies,
        series: s.Series,
        instances: s.Instances,
        modalities: s.Modalities,
        recent: (s.Recent as any[]).map(fromList),
      };
    },
    placeholderData: keepPreviousData,
  });
}

/** The filters' choices, with how many studies each has, over every study or one patient's. */
export function useStudyFacets(patient: string | null) {
  return useQuery({
    queryKey: ["studies", "facets", patient],
    queryFn: async (): Promise<StudyFacets> => {
      const url = new URL("api/studies/facets", location.href);
      url.hash = "";
      if (patient) url.searchParams.set("patient", patient);
      const f = await get(url);
      return { modalities: f.Modalities, sexes: f.Sexes, dates: f.Dates };
    },
  });
}

/** Every study of one patient, newest first. */
export function usePatientStudies(patient: string | null) {
  return useQuery({
    queryKey: ["studies", "patient", patient],
    enabled: !!patient,
    queryFn: async () => ((await get(search({ patient, limit: 1000, sort: "date", desc: true }))).Studies as any[]).map(fromList),
  });
}

/** Words typed anywhere (⌘K): the newest studies that have them all. */
export function useStudySearch(q: string) {
  return useQuery({
    queryKey: ["studies", "search", q],
    enabled: q.trim().length > 0,
    queryFn: async () => ((await get(search({ q, limit: 30, sort: "date", desc: true }))).Studies as any[]).map(fromList),
    placeholderData: keepPreviousData,
  });
}

/** One study by ID, whether or not it is on the page shown (a link can name any). */
export function useStudy(id: string | null, known: Study | undefined) {
  return useQuery({
    queryKey: ["studies", "one", id, !!known],
    enabled: !!id && !known,
    queryFn: async (): Promise<Study> => {
      const [s, series] = await Promise.all([api<any>(`studies/${id}`), api<any[]>(`studies/${id}/series`)]);
      return fromList({
        ...s,
        Modalities: [...new Set(series.map((x) => x.MainDicomTags.Modality).filter(Boolean))].sort(),
        Series: series.length,
        Instances: series.reduce((n, x) => n + (x.Instances?.length ?? 0), 0),
      });
    },
    initialData: known,
  });
}

export function useRefreshStudies() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: ["studies"] });
}

export async function deleteStudies(ids: string[]) {
  const failed: string[] = [];
  for (const id of ids) {
    try {
      await api(`studies/${id}`, { method: "DELETE" });
    } catch (e: any) {
      if (e.status !== 404) failed.push(e.message);
    }
  }
  return failed;
}

export function sortSeries<T extends { MainDicomTags: Record<string, any> }>(series: T[]) {
  return [...series].sort((a, b) => Number(a.MainDicomTags.SeriesNumber ?? 1e9) - Number(b.MainDicomTags.SeriesNumber ?? 1e9));
}

export function useStudySeries(id: string | null) {
  return useQuery({
    queryKey: ["series", id],
    enabled: !!id,
    queryFn: async () => {
      const [series, stats] = await Promise.all([api<any[]>(`studies/${id}/series`), api<any>(`studies/${id}/statistics`)]);
      return { series: sortSeries(series), stats };
    },
  });
}
