// The viewer's measurements on the server, one list per study (by Study Instance UID), so a
// study opens again with them, in any browser. The lists are Cornerstone's annotations as they
// are, with the server's DICOMweb address written as a placeholder (image IDs hold it; a
// server reached at another address must still find its images), without the statistics,
// which are worked out again on drawing, and without the pointer's hover over them.

import { WEB } from "@/lib/api";
import { authFetch } from "@/lib/session";

const HERE = "{dicomweb}/";
const LEFT_OUT = new Set(["cachedStats", "highlighted"]);
const url = (study: string) => new URL(`api/annotations/${encodeURIComponent(study)}`, location.href);

export async function fetchAnnotations(study: string): Promise<any[]> {
  const response = await authFetch(url(study), { cache: "no-store" });
  if (!response.ok) return [];
  const text = (await response.text()).replaceAll(JSON.stringify(HERE).slice(1, -1), JSON.stringify(WEB.href).slice(1, -1));
  const list = JSON.parse(text);
  if (!Array.isArray(list)) return [];
  return list
    .filter((a) => a && typeof a.annotationUID === "string" && a.metadata?.toolName)
    .map((a) => ({ ...a, highlighted: false }));
}

export async function saveAnnotations(study: string, list: any[]) {
  const text = JSON.stringify(list, (key, value) => (LEFT_OUT.has(key) ? undefined : value)).replaceAll(
    JSON.stringify(WEB.href).slice(1, -1),
    JSON.stringify(HERE).slice(1, -1),
  );
  const response = await authFetch(url(study), { method: "PUT", headers: { "Content-Type": "application/json" }, body: text });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      message = (await response.json()).Message || message;
    } catch {}
    throw new Error(message);
  }
}
