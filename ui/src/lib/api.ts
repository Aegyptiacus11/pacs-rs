// Where the server is, requests to it, and DICOM values formatted for people.

export const config: { dicomweb: string; version: string } = await fetch("config.json").then((r) => r.json());
/** The REST API: the server's root, next to this page's folder. */
export const ROOT = new URL("../", location.href);
/** DICOMweb, wherever the server serves it. */
export const WEB = new URL(config.dicomweb.replace(/^\/+|\/+$/g, "") + "/", ROOT);

export class HttpError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function api<T = any>(path: string, options: RequestInit = {}, base: URL = ROOT): Promise<T> {
  const response = await fetch(new URL(path, base), options);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      message = body.Message || body.Details || message;
    } catch {}
    throw new HttpError(response.status, message);
  }
  return (response.headers.get("content-type") || "").includes("json") ? response.json() : (response as T);
}

export const url = (path: string) => new URL(path, ROOT).href;

// ---- DICOM values ----

/** DOE^JANE -> Jane Doe */
export function personName(pn?: string) {
  if (!pn) return "Unnamed";
  const [family = "", given = "", middle = ""] = String(pn).split("^");
  const cap = (s: string) => s.toLowerCase().replace(/(^|[\s'-])\p{L}/gu, (m) => m.toUpperCase());
  return [given, middle, family].filter(Boolean).map(cap).join(" ") || "Unnamed";
}
export function initials(pn?: string) {
  const [family = "", given = ""] = String(pn || "").split("^");
  return ((given[0] || "") + (family[0] || "")).toUpperCase() || "?";
}
export function parseDate(da?: string) {
  return /^\d{8}$/.test(da || "") ? new Date(Number(da!.slice(0, 4)), Number(da!.slice(4, 6)) - 1, Number(da!.slice(6, 8))) : null;
}
const dateFormat = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
export function formatDate(da?: string) {
  const d = parseDate(da);
  return d ? dateFormat.format(d) : "—";
}
export function isoDate(da?: string) {
  return parseDate(da) ? `${da!.slice(0, 4)}-${da!.slice(4, 6)}-${da!.slice(6, 8)}` : "";
}
export function formatTime(tm?: string) {
  return /^\d{4}/.test(tm || "") ? `${tm!.slice(0, 2)}:${tm!.slice(2, 4)}` : "";
}
export function age(birth?: string, at?: string) {
  const b = parseDate(birth);
  if (!b) return null;
  const ref = parseDate(at) || new Date();
  let years = ref.getFullYear() - b.getFullYear();
  if (ref.getMonth() < b.getMonth() || (ref.getMonth() === b.getMonth() && ref.getDate() < b.getDate())) years--;
  return years >= 0 ? years : null;
}
export function sexAge(sex?: string, birth?: string, at?: string) {
  const a = age(birth, at);
  return [sex && sex !== "O" ? sex : null, a != null ? `${a}y` : null].filter(Boolean).join(" · ");
}
export function bytes(n: number | string) {
  let x = Number(n);
  if (!isFinite(x)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024;
    i++;
  }
  return `${x < 10 && i ? x.toFixed(1) : Math.round(x)} ${units[i]}`;
}
export function plural(n: number, word: string, words = `${word}s`) {
  return `${n.toLocaleString()} ${n === 1 ? word : words}`;
}
