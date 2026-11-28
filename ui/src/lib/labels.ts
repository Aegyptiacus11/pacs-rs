// Labels on studies: free words put on a study to group and find it (a teaching file, a
// cohort, a follow-up), through the REST API's /studies/{id}/labels. The study list shows
// them on each row and filters by them (/ui/api/studies?label=).

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

/** The server's rule: 1 to 64 ASCII letters, digits, `_` and `-`. */
export const LABEL = /^[A-Za-z0-9_-]{1,64}$/;

/** Why `text` cannot be a label, if it cannot. */
export function notALabel(text: string): string | null {
  if (!text) return null;
  if (text.length > 64) return "At most 64 characters";
  if (!LABEL.test(text)) return "Letters, digits, - and _ only";
  return null;
}

/** Every label in use, for the suggestions. */
export function useAllLabels(enabled = true) {
  return useQuery({
    queryKey: ["studies", "labels"],
    enabled,
    queryFn: () => api<string[]>("tools/labels"),
  });
}

const path = (study: string, label: string) => `studies/${study}/labels/${encodeURIComponent(label)}`;

export async function addLabel(study: string, label: string) {
  await api(path(study, label), { method: "PUT" });
}

export async function removeLabel(study: string, label: string) {
  await api(path(study, label), { method: "DELETE" });
}

/** The label put on each study, one after the other; how many it went on, and the first
 * error if one failed. */
export async function labelAll(studies: string[], label: string) {
  let done = 0;
  for (const id of studies) {
    try {
      await addLabel(id, label);
      done++;
    } catch (e) {
      // the ones before keep it; the caller says which failed
      return { done, error: e as Error };
    }
  }
  return { done, error: null };
}
