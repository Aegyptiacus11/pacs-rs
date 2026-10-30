// Shared helpers: where the server is, DOM building, requests, DICOM value formatting.

export const config = await fetch("config.json").then((r) => r.json());
export const ROOT = new URL("../", location.href);
export const WEB = new URL(config.dicomweb.replace(/^\/+|\/+$/g, "") + "/", ROOT);

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  el.append(...children.flat(Infinity).filter((c) => c != null && c !== false));
  return el;
}

export function searchIcon() {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("viewBox", "0 0 16 16");
  s.setAttribute("aria-hidden", "true");
  s.innerHTML = '<circle cx="7" cy="7" r="4.5"/><path d="m10.5 10.5 3.5 3.5"/>';
  return s;
}

export class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

export async function api(path, options = {}, base = ROOT) {
  const response = await fetch(new URL(path, base), options);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      message = body.Message || body.Details || message;
    } catch {}
    throw new HttpError(response.status, message);
  }
  return (response.headers.get("content-type") || "").includes("json") ? response.json() : response;
}

export function toast(message, kind = "") {
  const el = h("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" }, message);
  document.getElementById("toasts").append(el);
  setTimeout(() => el.remove(), kind === "error" ? 7000 : 2500);
}

export function confirmDelete(title, text) {
  const dialog = document.getElementById("confirm");
  document.getElementById("confirm-title").textContent = title;
  document.getElementById("confirm-text").textContent = text;
  dialog.returnValue = "";
  dialog.showModal();
  return new Promise((resolve) =>
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "ok"), { once: true }),
  );
}

export function copyable(text) {
  return h(
    "button",
    {
      class: "copy",
      type: "button",
      title: "Click to copy",
      onclick: async () => {
        await navigator.clipboard?.writeText(text);
        toast("Copied");
      },
    },
    text || "—",
  );
}

// DICOM values, as a workstation shows them
export function personName(pn) {
  if (!pn) return "—";
  const [family = "", given = "", middle = ""] = String(pn).split("^");
  return [family, [given, middle].filter(Boolean).join(" ")].filter(Boolean).join(", ") || "—";
}
export function parseDate(da) {
  return /^\d{8}$/.test(da || "") ? new Date(Number(da.slice(0, 4)), Number(da.slice(4, 6)) - 1, Number(da.slice(6, 8))) : null;
}
export function dicomDate(da) {
  return parseDate(da) ? `${da.slice(0, 4)}-${da.slice(4, 6)}-${da.slice(6, 8)}` : da || "—";
}
export function dicomTime(tm) {
  return /^\d{4}/.test(tm || "") ? `${tm.slice(0, 2)}:${tm.slice(2, 4)}` : "";
}
export function age(birth, at) {
  const b = parseDate(birth);
  if (!b) return "";
  const ref = parseDate(at) || new Date();
  let years = ref.getFullYear() - b.getFullYear();
  if (ref.getMonth() < b.getMonth() || (ref.getMonth() === b.getMonth() && ref.getDate() < b.getDate())) years--;
  return years >= 0 ? `${String(years).padStart(3, "0")}Y` : "";
}
export function sexAge(sex, birth, at) {
  return [sex || "-", age(birth, at)].filter(Boolean).join(" ");
}
export function bytes(n) {
  n = Number(n);
  if (!isFinite(n)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n < 10 && i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}
export function plural(n, word, words = `${word}s`) {
  return `${n.toLocaleString()} ${n === 1 ? word : words}`;
}
export const longDate = (da) =>
  parseDate(da)?.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" }) ?? "No date";

// the status line: a few facts on the left, keyboard hints on the right
export function setStatus(facts = [], hints = []) {
  document.getElementById("status").replaceChildren(
    ...facts.map((f) => h("span", {}, f)),
    h("span", { class: "end" }, hints.map(([key, what]) => h("span", {}, h("kbd", {}, key), " ", what))),
  );
}


export function isTyping(e) {
  return e.target.matches?.("input, select, textarea") || e.ctrlKey || e.metaKey || e.altKey || document.querySelector("dialog[open]");
}
