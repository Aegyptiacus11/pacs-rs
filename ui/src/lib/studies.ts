// The archive's studies and patients, as the lists show them: the REST API gives public IDs,
// QIDO-RS gives modalities and counts; they meet on Study Instance UID.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, WEB } from "@/lib/api";

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
  updated: string;
};

export type Patient = {
  id: string;
  name: string;
  patientId: string;
  sex: string;
  birth: string;
  studies: Study[];
  last: string;
  modalities: string[];
  instances: number;
};

async function everyPage<T>(fetchPage: (at: number, size: number) => Promise<T[]>, size: number) {
  const out: T[] = [];
  for (let at = 0; ; at += size) {
    const page = await fetchPage(at, size);
    out.push(...page);
    if (page.length < size) return out;
  }
}

async function fetchStudies(): Promise<Study[]> {
  const [rest, qido] = await Promise.all([
    everyPage<any>((at, n) => api(`studies?expand&since=${at}&limit=${n}`), 500),
    everyPage<any>((at, n) => api(`studies?offset=${at}&limit=${n}`, { headers: { Accept: "application/dicom+json" } }, WEB), 1000),
  ]);
  const byUid = new Map(qido.map((q) => [q["0020000D"]?.Value?.[0], q]));
  return rest.map((s) => {
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
}

export function useStudies() {
  return useQuery({ queryKey: ["studies"], queryFn: fetchStudies, staleTime: 15_000 });
}

export function usePatients() {
  const studies = useStudies();
  const patients = new Map<string, Patient>();
  for (const s of studies.data ?? []) {
    const p = patients.get(s.patient) ?? { id: s.patient, name: s.name, patientId: s.patientId, sex: s.sex, birth: s.birth, studies: [], last: "", modalities: [], instances: 0 };
    p.studies.push(s);
    p.instances += s.instances;
    if (s.date > p.last) p.last = s.date;
    for (const m of s.modalities) if (!p.modalities.includes(m)) p.modalities.push(m);
    patients.set(s.patient, p);
  }
  return { ...studies, data: studies.data ? [...patients.values()] : undefined };
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
