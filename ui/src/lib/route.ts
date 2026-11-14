// Hash routes: #/studies[?patient=ID][&open=STUDY], #/patients[?open=PATIENT], #/users,
// #/view/STUDY[/SERIES][?compare=STUDY]: without a series the viewer hangs the study by
// protocol; with another study, the two side by side.

import { useSyncExternalStore } from "react";

export type Route =
  | { page: "studies"; patient: string | null; open: string | null }
  | { page: "patients"; open: string | null }
  | { page: "users" }
  | { page: "view"; study: string; series: string | null; compare: string | null };

function parse(): Route {
  const [path, query = ""] = (location.hash.replace(/^#/, "") || "/studies").split("?");
  const params = new URLSearchParams(query);
  const view = /^\/view\/([\w-]+)(?:\/([\w-]+))?$/.exec(path);
  if (view) return { page: "view", study: view[1], series: view[2] ?? null, compare: params.get("compare") };
  if (path === "/users") return { page: "users" };
  if (path === "/patients") return { page: "patients", open: params.get("open") };
  return { page: "studies", patient: params.get("patient"), open: params.get("open") };
}

let current = parse();
const listeners = new Set<() => void>();
addEventListener("hashchange", () => {
  current = parse();
  listeners.forEach((l) => l());
});

export function useRoute() {
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => current,
  );
}

export const go = (hash: string) => (location.hash = hash);
export const href = {
  studies: (patient?: string | null, open?: string | null) => {
    const q = new URLSearchParams();
    if (patient) q.set("patient", patient);
    if (open) q.set("open", open);
    return `#/studies${q.size ? `?${q}` : ""}`;
  },
  patients: (open?: string | null) => (open ? `#/patients?open=${encodeURIComponent(open)}` : "#/patients"),
  view: (study: string, series?: string | null, compare?: string | null) =>
    (series ? `#/view/${study}/${series}` : `#/view/${study}`) + (compare ? `?compare=${encodeURIComponent(compare)}` : ""),
};
