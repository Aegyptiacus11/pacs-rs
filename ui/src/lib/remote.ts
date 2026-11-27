// Other archives, over the DICOM network: the remote AEs (those of PACS_DIMSE_REMOTES, and
// those an administrator adds, changes and removes), a C-ECHO to each, a C-FIND of one's
// studies, and the jobs that pull studies in (C-MOVE, or C-GET) and send them out (C-STORE).
// Every call is one of the REST API (/modalities, /jobs).

import { useQuery } from "@tanstack/react-query";
import { api, HttpError, ROOT } from "@/lib/api";
import { authFetch } from "@/lib/session";

/** A remote AE; `fixed` when it comes from PACS_DIMSE_REMOTES, changed there only. */
export type RemoteAe = { id: string; aet: string; host: string; port: number; tls: boolean; mayReceive: boolean; fixed: boolean };

export function useRemotes(enabled = true) {
  return useQuery({
    queryKey: ["remotes"],
    enabled,
    queryFn: async (): Promise<RemoteAe[]> => {
      const byId = await api<Record<string, { AET: string; Host: string; Port: number; TLS?: boolean; MayReceive?: boolean; Fixed?: boolean }>>("modalities?expand");
      return Object.entries(byId)
        .map(([id, ae]) => ({ id, aet: ae.AET, host: ae.Host, port: ae.Port, tls: !!ae.TLS, mayReceive: !!ae.MayReceive, fixed: !!ae.Fixed }))
        .sort((a, b) => a.id.localeCompare(b.id));
    },
  });
}

/** Why `id` cannot name a remote AE, as the server checks it, if it cannot. */
export function badRemoteId(id: string) {
  return /^[A-Za-z0-9_-]{1,64}$/.test(id) ? null : "1 to 64 letters, digits, - and _";
}

/** Why `aet` cannot be an AE title (PS3.5 6.2, AE), if it cannot. */
export function badAeTitle(aet: string) {
  const t = aet.trim();
  if (!t) return "Required";
  if (aet.length > 16) return "At most 16 characters";
  return /^[\x20-\x5b\x5d-\x7e]+$/.test(aet) ? null : "No backslash, accent or control character";
}

/** Adds the remote AE `id`, or changes one added before; answers it as it is now. */
export function putRemote(id: string, ae: { aet: string; host: string; port: number; tls: boolean; mayReceive: boolean }) {
  return api(`modalities/${encodeURIComponent(id)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ AET: ae.aet.trim(), Host: ae.host.trim(), Port: ae.port, TLS: ae.tls, MayReceive: ae.mayReceive }),
  });
}

export function deleteRemote(id: string) {
  return api(`modalities/${encodeURIComponent(id)}`, { method: "DELETE" });
}

/** A C-ECHO: how long it took, or why it failed. */
export function useEcho(id: string) {
  return useQuery({
    queryKey: ["remotes", id, "echo"],
    queryFn: () => api<{ ElapsedMs: number }>(`modalities/${encodeURIComponent(id)}/echo`, { method: "POST" }),
    retry: false,
    staleTime: 60_000,
  });
}

export type RemoteQuery = { name: string; patientId: string; accession: string; from: string; to: string; modality: string };
export const emptyQuery: RemoteQuery = { name: "", patientId: "", accession: "", from: "", to: "", modality: "" };

export type RemoteStudy = {
  uid: string;
  patientId: string;
  name: string;
  birth: string;
  sex: string;
  date: string;
  time: string;
  description: string;
  accession: string;
  modalities: string[];
  series: number | null;
  instances: number | null;
};

const LIMIT = 200;

/** A query's keys as GET /modalities/{id}/studies takes them; a name or an ID is matched as
 * a prefix unless it already has a wild card. */
export function searchParams(q: RemoteQuery) {
  const p = new URLSearchParams();
  const prefix = (v: string) => (/[*?]/.test(v) ? v : `${v}*`);
  if (q.name.trim()) p.set("PatientName", prefix(q.name.trim().replace(/\s+/g, "*")));
  if (q.patientId.trim()) p.set("PatientID", prefix(q.patientId.trim()));
  if (q.accession.trim()) p.set("AccessionNumber", q.accession.trim());
  const from = q.from.replaceAll("-", "");
  const to = q.to.replaceAll("-", "");
  if (from || to) p.set("StudyDate", from === to ? from : `${from}-${to}`);
  if (q.modality) p.set("ModalitiesInStudy", q.modality);
  p.set("limit", String(LIMIT));
  return p;
}

const value = (ds: any, tag: string) => ds?.[tag]?.Value ?? [];
const text = (ds: any, tag: string) => {
  const v = value(ds, tag)[0];
  return v == null ? "" : typeof v === "object" ? String(v.Alphabetic ?? "") : String(v);
};
const count = (ds: any, tag: string) => (value(ds, tag).length ? Number(value(ds, tag)[0]) : null);

export function fromDicomJson(ds: any): RemoteStudy {
  return {
    uid: text(ds, "0020000D"),
    patientId: text(ds, "00100020"),
    name: text(ds, "00100010"),
    birth: text(ds, "00100030"),
    sex: text(ds, "00100040"),
    date: text(ds, "00080020"),
    time: text(ds, "00080030"),
    description: text(ds, "00081030"),
    accession: text(ds, "00080050"),
    modalities: value(ds, "00080061").map(String),
    series: count(ds, "00201206"),
    instances: count(ds, "00201208"),
  };
}

/** The search, asked when `q` is given. The archive's Warning header (more studies than the
 * limit, keys it did not match on, an early end) comes back with the studies. */
export function useRemoteSearch(id: string | null, q: RemoteQuery | null) {
  return useQuery({
    queryKey: ["remotes", id, "studies", q],
    enabled: !!id && !!q,
    retry: false,
    queryFn: async () => {
      const response = await authFetch(new URL(`modalities/${encodeURIComponent(id!)}/studies?${searchParams(q!)}`, ROOT), {
        headers: { Accept: "application/dicom+json" },
        cache: "no-store",
      });
      if (!response.ok) {
        let message = `${response.status} ${response.statusText}`;
        try {
          const body = await response.json();
          message = [body.Message, body.Details].filter(Boolean).join(": ") || message;
        } catch {}
        throw new HttpError(response.status, message);
      }
      const studies = ((await response.json()) as any[]).map(fromDicomJson);
      return { studies, warning: response.headers.get("Warning"), limit: LIMIT };
    },
  });
}

export type Started = { ID: string; Path: string };

/** Pulls studies in, as a job; to a project's AE title when one is named. */
export function pull(id: string, studies: string[], project?: string | null) {
  return api<Started>(`modalities/${encodeURIComponent(id)}/move`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ StudyInstanceUID: studies, Asynchronous: true, ...(project ? { Project: project } : {}) }),
  });
}

/** Sends studies out, as a job. */
export function send(id: string, studies: string[]) {
  return api<Started>(`modalities/${encodeURIComponent(id)}/store`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ StudyInstanceUID: studies, Asynchronous: true }),
  });
}

export type Job = {
  ID: string;
  Type: "C-MOVE" | "C-GET" | "C-STORE" | "WRITE-BACK" | string;
  Remote: string;
  PatientID: string | null;
  StudyInstanceUIDs: string[];
  State: "Running" | "Success" | "Warning" | "Cancelled" | "Failure" | string;
  Progress: number | null;
  Remaining: number | null;
  Completed: number | null;
  Failed: number | null;
  Warning: number | null;
  FailedSOPInstanceUIDs: string[];
  Error: string | null;
  ElapsedMs: number;
  Restarts: number;
};

/** Every job, newest first; asked again every two seconds while one runs. */
export function useJobs() {
  return useQuery({
    queryKey: ["jobs"],
    queryFn: async () => {
      const ids = await api<string[]>("jobs");
      const jobs = await Promise.all(ids.map((id) => api<Job>(`jobs/${encodeURIComponent(id)}`)));
      return jobs.reverse();
    },
    refetchInterval: (query) => (query.state.data?.some((j) => j.State === "Running") ? 2_000 : 15_000),
  });
}

export function cancelJob(id: string) {
  return api<Job>(`jobs/${encodeURIComponent(id)}/cancel`, { method: "POST" });
}

/** "Pull", "Send" or "Write back", for people. */
export function jobKind(job: Job) {
  return job.Type === "C-STORE" ? "Send" : job.Type === "WRITE-BACK" ? "Write back" : "Pull";
}
