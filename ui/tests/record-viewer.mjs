// Records what the viewer asks of the server for one data set of tests/data (study, series,
// instances, DICOMweb metadata and frames, protocols, annotations) as a HAR in tests/fixtures,
// which the viewer's browser tests replay. Needs dist/ (npm run build) and a pacs-server with
// no login that holds only that data set:
//
//   docker run -d --rm -p 127.0.0.1:18092:8080 -e PACS_LISTEN=0.0.0.0:8080 ghcr.io/aegyptiacus11/pacs-rs:<version>
//   for f in tests/data/prostate-dce/*.dcm; do curl -fsS --data-binary @$f http://127.0.0.1:18092/instances; done
//   PACS_URL=http://127.0.0.1:18092 node tests/record-viewer.mjs prostate-dce
//
// - prostate-dce -> fixtures/viewer-dce.har: the series, then a sagittal plane beside it.
// - kidney-seg (tests/data/kidney-seg/ct/*.dcm, seg.dcm and rtstruct.dcm) ->
//   fixtures/viewer-kidney.har: the CT, its SEG and RTSTRUCT laid over it, then MPR. The SEG
//   and RTSTRUCT files themselves are left out: the tests serve them from tests/data.
//
// Stops with exit 1 when the server does not hold that data set, or a request fails.
import { existsSync } from "node:fs";
import path from "node:path";
import { chromium } from "@playwright/test";

const SETS = {
  "prostate-dce": { har: "viewer-dce.har", series: ["DCE 262 pre"], open: "DCE 262 pre" },
  "kidney-seg": {
    har: "viewer-kidney.har",
    series: ["Recon 2: +C", "AIMI kidney tumor cyst AI segmentation", "Pre-Dose, RIGHT KIDNEY - 1"],
    open: "Recon 2: +C",
  },
};
const set = SETS[process.argv[2]];
const base = process.env.PACS_URL;
if (!set) {
  console.error(`which data set: ${Object.keys(SETS).join(" or ")}`);
  process.exit(1);
}
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
  console.error(`the server holds ${studies.length} studies, not the one of tests/data/${process.argv[2]}`);
  process.exit(1);
}
const series = await (await fetch(new URL(`studies/${studies[0]}/series`, base))).json();
const names = series.map((s) => s.MainDicomTags.SeriesDescription).sort();
if (JSON.stringify(names) !== JSON.stringify([...set.series].sort())) {
  console.error(`the server's series are ${JSON.stringify(names)}, not ${JSON.stringify(set.series)}`);
  process.exit(1);
}
const opened = series.find((s) => s.MainDicomTags.SeriesDescription === set.open);

const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  recordHar: {
    path: path.join(here, "fixtures", set.har),
    content: "embed",
    // the built interface, and whole files, are not recorded
    urlFilter: /^http:\/\/pacs\.test\/(?!ui\/(?!config\.json|api\/))(?!instances\/[\w-]+\/file$)/,
  },
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
await page.goto(`http://pacs.test/ui/#/view/${studies[0]}/${opened.ID}`);
await page.getByText(/W \d+ {2}L \d+/).first().waitFor();
if (process.argv[2] === "prostate-dce") {
  // a second viewport, a sagittal plane of the volume: every frame is asked for
  await page.keyboard.press("n");
  await page.locator(".ov-tl", { hasText: "Sagittal" }).waitFor();
} else {
  for (const name of ["AIMI kidney", "Pre-Dose"]) await page.getByRole("button", { name: new RegExp(name) }).click();
  await page.getByRole("list", { name: /^Regions of/ }).waitFor();
  await page.keyboard.press("x");
  await page.locator(".ov-tl", { hasText: "Sagittal" }).waitFor();
}
await page.waitForTimeout(3000);
await context.close();
await browser.close();
if (failed.length) {
  console.error(failed.join("\n"));
  process.exit(1);
}
console.log(`fixtures/${set.har}`);
