// What is typed in the top bar's search, one text per list: the bar writes it, the page that
// lists reads it and filters by it.

import { useSyncExternalStore } from "react";

export type Searched = "studies" | "patients";

const texts: Record<Searched, string> = { studies: "", patients: "" };
const listeners = new Set<() => void>();

export function setSearch(page: Searched, text: string) {
  if (texts[page] === text) return;
  texts[page] = text;
  listeners.forEach((l) => l());
}

export function useSearch(page: Searched) {
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => texts[page],
  );
}
