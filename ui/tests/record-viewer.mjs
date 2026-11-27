// Records what the viewer asks of the server for the series of tests/data/prostate-dce (study,
// series, instances, DICOMweb metadata and frames, protocols, annotations) as
// tests/fixtures/viewer-dce.har, which the viewer's browser tests replay. Needs dist/ (npm run
// build) and a pacs-server with no login that holds only that series:
//
//   docker run -d --rm -p 127.0.0.1:18092:8080 -e PACS_LISTEN=0.0.0.0:8080 ghcr.io/aegyptiacus11/pacs-rs:<version>
//   for f in tests/data/prostate-dce/*.dcm; do curl -fsS --data-binary @$f http://127.0.0.1:18092/instances; done
//   PACS_URL=http://127.0.0.1:18092 node tests/record-viewer.mjs
//
// Stops with exit 1 when the server does not hold that one series, or a request fails.
import { existsSync } from "node:fs";
import path from "node:path";
import { chromium } from "@playwright/test";

const base = process.env.PACS_URL;
if (!base) {
  console.error("PACS_URL is not set");
  process.exit(1);
}
const here = path.dirname(new URL(import.meta.url).pathname);
const DIST = path.resolve(here, "../dist");
if (!existsSync(path.join(DIST, "index.html"))) {
  console.error("dist/ is missing: run `npm run build` first");
  process.exit(1);
}

const studies = await (await fetch(new URL("studies", base))).json();
if (studies.length !== 1) {
  console.error(`the server holds ${studies.length} studies, not the one of tests/data/prostate-dce`);
  process.exit(1);
}
const series = await (await fetch(new URL(`studies/${studies[0]}/series`, base))).json();
if (series.length !== 1 || series[0].MainDicomTags.SeriesDescription !== "DCE 262 pre") {
  console.error("the server's series is not DCE 262 pre");
  process.exit(1);
}

const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  recordHar: { path: path.join(here, "fixtures/viewer-dce.har"), content: "embed", urlFilter: /^http:\/\/pacs\.test\/(?!ui\/(?!config\.json|api\/))/ },
});
const page = await context.newPage();
const failed = [];
await page.route("http://pacs.test/**", async (route) => {
  const url = new URL(route.request().url());
  const file = url.pathname.startsWith("/ui/") && url.pathname !== "/ui/config.json" && !url.pathname.startsWith("/ui/api/")
    ? path.join(DIST, url.pathname.slice(4) || "index.html")
    : null;
  if (file && existsSync(file)) return route.fulfill({ path: file });
  const response = await route.fetch({ url: new URL(url.pathname.slice(1) + url.search, base).href });
  if (response.status() >= 400) failed.push(`${route.request().method()} ${url.pathname}: ${response.status()}`);
  return route.fulfill({ response });
});
await page.goto(`http://pacs.test/ui/#/view/${studies[0]}/${series[0].ID}`);
await page.getByText(/W \d+ {2}L \d+/).first().waitFor();
// a second viewport, a sagittal plane of the volume: every frame is asked for
await page.keyboard.press("n");
await page.locator(".ov-tl", { hasText: "Sagittal" }).waitFor();
await page.waitForTimeout(3000);
await context.close();
await browser.close();
if (failed.length) {
  console.error(failed.join("\n"));
  process.exit(1);
}
console.log("fixtures/viewer-dce.har");
