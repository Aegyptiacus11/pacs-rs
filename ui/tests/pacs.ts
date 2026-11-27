// A pacs-server stand-in for the browser tests. Every request of the page goes to
// http://pacs.test and is answered here: /ui/ from the built interface (dist/), the REST API
// from the answers a test gives, by default the ones recorded in fixtures/ (record.mjs).
// A request nobody answers fails the test, so a page that starts asking for something new
// is noticed.
import { readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { test as base, expect, type Page, type Route } from "@playwright/test";

export const ORIGIN = "http://pacs.test";
const DIST = path.resolve(import.meta.dirname, "../dist");
const FIXTURES = path.resolve(import.meta.dirname, "fixtures");

export function fixture(name: string): any {
  return JSON.parse(readFileSync(path.join(FIXTURES, name), "utf8"));
}

const TYPES: Record<string, string> = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".woff2": "font/woff2",
};

type Answer = { status?: number; json?: unknown; body?: string; headers?: Record<string, string> };
type Handler = (route: Route, url: URL) => Answer | Promise<Answer>;

export class Pacs {
  private handlers: { method: string; path: string | RegExp; handler: Handler }[] = [];
  /** Requests nobody answered: the test fails if any is left. */
  readonly unanswered: string[] = [];
  /** Every API request answered, as `METHOD /path?query`. */
  readonly asked: string[] = [];

  constructor() {
    this.on("GET", "/ui/config.json", () => ({ json: fixture("config.json") }));
    this.on("GET", "/healthz", () => ({ body: "ok" }));
    this.on("GET", "/ui/api/studies", (_, url) =>
      ({ json: url.searchParams.get("summary") === "true" ? fixture("studies-summary.json") : fixture("studies.json") }));
    this.on("GET", "/ui/api/studies/facets", () => ({ json: fixture("studies-facets.json") }));
    this.on("GET", "/ui/api/patients", () => ({ json: fixture("patients.json") }));
    this.on("GET", "/ui/api/server", () => ({ json: fixture("server.json") }));
  }

  /** Answers `method path` this way from now on; the latest one given wins. */
  on(method: string, path: string | RegExp, handler: Handler) {
    this.handlers.unshift({ method, path, handler });
    return this;
  }

  async attach(page: Page) {
    await page.route(`${ORIGIN}/**`, async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      const file = this.file(url.pathname);
      if (request.method() === "GET" && file) {
        return route.fulfill({ path: file, contentType: TYPES[path.extname(file)] ?? "application/octet-stream" });
      }
      const match = this.handlers.find(
        (h) => h.method === request.method() && (typeof h.path === "string" ? h.path === url.pathname : h.path.test(url.pathname)),
      );
      if (!match) {
        this.unanswered.push(`${request.method()} ${url.pathname}${url.search}`);
        return route.fulfill({ status: 599, body: "no answer in the test" });
      }
      this.asked.push(`${request.method()} ${url.pathname}${url.search}`);
      const answer = await match.handler(route, url);
      return route.fulfill({
        status: answer.status ?? 200,
        headers: answer.headers,
        contentType: answer.json !== undefined ? "application/json" : "text/plain",
        body: answer.json !== undefined ? JSON.stringify(answer.json) : (answer.body ?? ""),
      });
    });
  }

  /** The built interface's file for a path under /ui/, if there is one. */
  private file(pathname: string) {
    if (!pathname.startsWith("/ui/")) return null;
    const rel = pathname.slice("/ui/".length) || "index.html";
    if (rel === "config.json") return null;
    const full = path.join(DIST, rel);
    return full.startsWith(DIST) && existsSync(full) ? full : null;
  }
}

export const test = base.extend<{ pacs: Pacs }>({
  pacs: async ({ page }, use) => {
    if (!existsSync(path.join(DIST, "index.html"))) throw new Error("dist/ is missing: run `npm run build` first");
    const pacs = new Pacs();
    await pacs.attach(page);
    await use(pacs);
    expect(pacs.unanswered, "requests no test answered").toEqual([]);
  },
});

/** How long the first image of a viewer may take: decoded and drawn through software WebGL,
 * with the other tests' viewers doing the same on the machine's cores. */
export const FIRST_IMAGE = { timeout: 20_000 };

export { expect };
