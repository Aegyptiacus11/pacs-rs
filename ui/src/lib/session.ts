// The session, when the server asks for a login (config.auth). The access token lives in
// memory only and goes in the Authorization header; the refresh token is an httpOnly cookie
// this page never sees, sent only to auth/*. On load the page asks for a fresh access token:
// that is how a session survives a reload. A 401 is answered by one refresh and one retry.
// Every refresh in a page shares one request: the server spends a refresh token on use, and
// a second one at the same moment would look like a stolen cookie.
//
// Without a login (config.auth false) everything here passes requests through untouched.

import { useSyncExternalStore } from "react";
import { config, ROOT } from "@/lib/config";

/** `role` is the id of the user's role; `permissions` what that role gives. */
export type User = { id: string; login: string; role: string; permissions: string[] };
type State = { status: "loading" | "out" | "in"; user: User | null };

export const authOn = config.auth === true;

let state: State = authOn ? { status: "loading", user: null } : { status: "in", user: null };
let token: string | null = null;
/** When the access token runs out, in milliseconds. */
let expires = 0;
let renewal: ReturnType<typeof setTimeout> | undefined;
let refreshing: Promise<boolean> | null = null;
const listeners = new Set<() => void>();

function set(next: State) {
  state = next;
  listeners.forEach((l) => l());
}

export function useSession() {
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => state,
  );
}

/** What the signed-in user may do; everything when the server asks for no login. */
export function may(right: "write" | "remote" | "delete" | "admin", user: User | null = state.user) {
  if (!authOn) return right !== "admin";
  if (!user) return false;
  return user.permissions.includes(right === "admin" ? "users" : right);
}

const endpoint = (name: string) => new URL(`auth/${name}`, ROOT);

async function detail(response: Response, fallback: string) {
  try {
    const body = await response.json();
    return String(body.detail || body.Message || fallback);
  } catch {
    return fallback;
  }
}

/** The moment a JWT expires, from its payload; a minute from now if it cannot be read. */
function expiry(jwt: string) {
  try {
    const payload = JSON.parse(atob(jwt.split(".")[1].replace(/-/g, "+").replace(/_/g, "/")));
    if (typeof payload.exp === "number") return payload.exp * 1000;
  } catch {}
  return Date.now() + 60_000;
}

function keep(access: string) {
  token = access;
  expires = expiry(access);
  clearTimeout(renewal);
  // a minute early, so the first request after an idle spell does not wait for a refresh
  renewal = setTimeout(() => void refresh(), Math.max(5_000, expires - Date.now() - 60_000));
}

function signedOut() {
  token = null;
  clearTimeout(renewal);
  if (state.status !== "out") set({ status: "out", user: null });
}

async function me(): Promise<User> {
  const response = await fetch(endpoint("me"), { headers: { Authorization: `Bearer ${token}` }, cache: "no-store" });
  if (!response.ok) throw new Error(await detail(response, "The account could not be read"));
  return response.json();
}

/** A fresh access token from the refresh cookie; false when the session is over. */
function refresh(): Promise<boolean> {
  refreshing ??= (async () => {
    try {
      const response = await fetch(endpoint("refresh"), { method: "POST", cache: "no-store" });
      if (response.status === 401) {
        signedOut();
        return false;
      }
      // the server is down or failing: the session may well be fine, so it is kept
      if (!response.ok) throw new Error(await detail(response, `${response.status} ${response.statusText}`));
      keep((await response.json()).access_token);
      if (!state.user) set({ status: "in", user: await me() });
      return true;
    } finally {
      refreshing = null;
    }
  })();
  return refreshing;
}

/** On load: restore the session if there is one. */
export async function restore() {
  if (!authOn) return;
  try {
    await refresh();
  } catch {
    // no answer: show the sign-in page, which says so when it is tried
    signedOut();
  }
}

export async function signIn(login: string, password: string) {
  const response = await fetch(endpoint("login"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email: login, password }),
  });
  if (response.status === 401) throw new Error("Wrong login or password.");
  if (!response.ok) throw new Error(await detail(response, "The server did not answer. Try again."));
  keep((await response.json()).access_token);
  set({ status: "in", user: await me() });
}

export async function signOut() {
  try {
    await fetch(endpoint("logout"), { method: "POST" });
  } finally {
    // whatever the server said, this page forgets the token
    signedOut();
  }
}

/** The Authorization header to send now, refreshed first if the token is about to run out. */
export async function authHeaders(): Promise<Record<string, string>> {
  if (!authOn) return {};
  if (!token || Date.now() > expires - 5_000) await refresh().catch(() => false);
  return token ? { Authorization: `Bearer ${token}` } : {};
}

/** fetch, signed in: with the header, and after a 401 one refresh and one retry. */
export async function authFetch(input: URL | string, init: RequestInit = {}): Promise<Response> {
  if (!authOn) return fetch(input, init);
  const send = async () => fetch(input, { ...init, headers: { ...Object.fromEntries(new Headers(init.headers)), ...(await authHeaders()) } });
  let response = await send();
  if (response.status === 401 && (await refresh().catch(() => false))) response = await send();
  if (response.status === 401) signedOut();
  return response;
}

/** `POST auth/password`: every session of the user ends, this one included. */
export async function changePassword(current: string, next: string) {
  const response = await authFetch(endpoint("password"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ current_password: current, new_password: next }),
  });
  if (response.status === 403) throw new Error("The current password is wrong.");
  if (!response.ok) throw new Error(await detail(response, "The password was not changed."));
  signedOut();
}

/** A URL the browser can follow by itself for `path` (relative to the server's root, with
 * its query): with a download token good for a minute when the server asks for a login. */
export async function downloadUrl(path: string): Promise<string> {
  const target = new URL(path, ROOT);
  if (!authOn) return target.href;
  const response = await authFetch(endpoint("download-token"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    // as the server sees it: from its root, whatever prefix a proxy adds
    body: JSON.stringify({ path: `/${path.replace(/^\/+/, "")}` }),
  });
  if (!response.ok) throw new Error(await detail(response, "The download could not be started."));
  target.searchParams.append("token", (await response.json()).token);
  return target.href;
}
