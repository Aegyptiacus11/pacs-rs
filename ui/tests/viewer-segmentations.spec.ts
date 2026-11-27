import { readFileSync } from "node:fs";
import path from "node:path";
import { test, expect } from "./pacs";

// tests/data/kidney-seg, as the viewer asked a pacs-server for it (record-viewer.mjs kidney-seg):
// the CT, then the SEG and RTSTRUCT, whose files are served here from tests/data
const HAR = path.join(import.meta.dirname, "fixtures/viewer-kidney.har");
const DATA = path.join(import.meta.dirname, "data/kidney-seg");
const VIEW = "#/view/9fe34720-0aef8f74-788ae8a7-74ce28b1-45495d68/23efc5ff-23e7ab69-fb9df9a0-5aca5ab6-0e406fbb";
const SEG_FILE = "/instances/3898956d-f2bc64c1-f5388ad5-290eeeb3-3a02f64b/file";
const RT_FILE = "/instances/6a7c4f55-44df335e-e352b676-1e8955f4-92bfcfbf/file";
const dicom = (name: string) => ({ body: readFileSync(path.join(DATA, name)) as any, headers: { "content-type": "application/dicom" } });

test.beforeEach(async ({ page, pacs }) => {
  pacs.on("GET", SEG_FILE, () => dicom("seg.dcm"));
  pacs.on("GET", RT_FILE, () => dicom("rtstruct.dcm"));
  pacs.on("PUT", /^\/ui\/api\/annotations\//, () => ({ status: 204 }));
  await page.routeFromHAR(HAR, { url: /^http:\/\/pacs\.test\/(?!ui\/(?!config\.json|api\/))(?!instances\/[\w-]+\/file$)/, notFound: "fallback" });
});

type Page = import("@playwright/test").Page;

async function open(page: Page) {
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText(/W \d+ {2}L \d+/);
}
const lay = (page: Page, name: RegExp) => page.getByRole("button", { name }).click();

/** How many pixels of the first viewport's mask layer are painted, and its box on screen. */
const painted = (page: Page) =>
  page.locator(".cell-masks").first().evaluate((c: HTMLCanvasElement) => {
    if (c.hidden) return { pixels: 0, box: null };
    const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
    let pixels = 0;
    for (let i = 3; i < d.length; i += 4) if (d[i]) pixels++;
    const r = c.getBoundingClientRect(), v = c.parentElement!.querySelector(".cs")!.getBoundingClientRect();
    return { pixels, box: { x: r.x - v.x, y: r.y - v.y, w: r.width, h: r.height, vw: v.width, vh: v.height } };
  });

test("a SEG lays its segments over the CT it was drawn on, exactly over its pixels", async ({ page }) => {
  await open(page);
  await lay(page, /AIMI kidney tumor cyst/);
  const panel = page.getByRole("region", { name: "Segmentations" });
  await expect(panel.getByRole("list", { name: /^Segments of/ }).getByRole("listitem")).toHaveText(["Kidney", "Mass", "Cyst"]);
  // 101 frames over 50 slices, of which this archive has 8
  await expect(panel).toContainText("77 of its frames lie on images this archive does not have.");
  const { pixels, box } = await painted(page);
  expect(pixels).toBeGreaterThan(1000);
  // the 512 x 512 image fills the viewport's height, centered: so does the mask layer
  const side = Math.min(box!.vw, box!.vh);
  expect(box!.w).toBeCloseTo(side, 0);
  expect(box!.h).toBeCloseTo(side, 0);
  expect(box!.x).toBeCloseTo((box!.vw - side) / 2, 0);
  expect(box!.y).toBeCloseTo((box!.vh - side) / 2, 0);
});

test("segments are hidden one by one or all, the fill changes, and × takes the SEG away", async ({ page }) => {
  await open(page);
  await lay(page, /AIMI kidney tumor cyst/);
  const panel = page.getByRole("region", { name: "Segmentations" });
  await expect.poll(async () => (await painted(page)).pixels).toBeGreaterThan(1000);
  const all = (await painted(page)).pixels;
  await panel.getByRole("button", { name: "Hide Kidney" }).click();
  await expect.poll(async () => (await painted(page)).pixels).toBeLessThan(all / 2);
  await panel.getByRole("button", { name: "Show Kidney" }).click();
  await expect.poll(async () => (await painted(page)).pixels).toBe(all);
  await panel.getByRole("button", { name: "Hide AIMI kidney tumor cyst AI segmentation" }).click();
  await expect.poll(async () => (await painted(page)).pixels).toBe(0);
  await panel.getByRole("button", { name: "Show AIMI kidney tumor cyst AI segmentation" }).click();
  // no fill: the outlines alone
  await panel.getByRole("slider", { name: "Fill opacity" }).fill("0");
  await expect.poll(async () => (await painted(page)).pixels).toBeLessThan(all / 4);
  await panel.getByRole("button", { name: "Take AIMI kidney tumor cyst AI segmentation away" }).click();
  await expect(page.getByRole("region", { name: "Segmentations" })).toHaveCount(0);
  await expect.poll(async () => (await painted(page)).pixels).toBe(0);
});

test("an RTSTRUCT draws its contour on its slice, and as a line across it in MPR", async ({ page }) => {
  await open(page);
  await lay(page, /Pre-Dose, RIGHT/);
  await expect(page.getByRole("list", { name: /^Regions of/ }).getByRole("listitem")).toHaveText(["RT KIDNEY - 1"]);
  const stack = page.locator(".cell-contours").first();
  await expect(stack.locator("polygon")).toHaveCount(1);
  await expect(stack.locator("polygon")).toHaveAttribute("stroke", "rgb(255,255,0)");
  // MPR: the axial view shows the contour of its slice, sagittal and coronal cut across them
  await lay(page, /AIMI kidney tumor cyst/);
  await page.keyboard.press("x");
  await expect(page.locator(".ov-tl", { hasText: "Sagittal" })).toBeVisible();
  // the axial view's slice has its contour; a click inside it brings the other planes there
  const axial = page.locator(".cell[data-plane=axial] .cell-contours polygon");
  await expect(axial).toHaveCount(1);
  const box = (await axial.boundingBox())!;
  for (const plane of ["sagittal", "coronal"]) await expect(page.locator(`.cell[data-plane=${plane}] .cell-contours polyline`)).toHaveCount(0);
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  for (const plane of ["sagittal", "coronal"]) await expect.poll(() => page.locator(`.cell[data-plane=${plane}] .cell-contours polyline`).count()).toBeGreaterThan(0);
  // the SEG goes to Cornerstone there: no error on the way
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.waitForTimeout(500);
  expect(errors).toEqual([]);
});

test("a SEG that cannot be read says why, and nothing is laid over the images", async ({ page, pacs }) => {
  // the same file, said to be RLE compressed
  const seg = readFileSync(path.join(DATA, "seg.dcm"));
  const at = seg.indexOf("1.2.840.10008.1.2.1\0");
  seg.write("1.2.840.10008.1.2.5\0", at, "latin1");
  pacs.on("GET", SEG_FILE, () => ({ body: seg as any, headers: { "content-type": "application/dicom" } }));
  await open(page);
  await lay(page, /AIMI kidney tumor cyst/);
  await expect(page.getByText("The segmentation could not be shown")).toBeVisible();
  await expect(page.getByText(/stored compressed \(1\.2\.840\.10008\.1\.2\.5\)/)).toBeVisible();
  await expect(page.getByRole("region", { name: "Segmentations" })).toHaveCount(0);
  expect((await painted(page)).pixels).toBe(0);
});

test("a segmentation file the server refuses says so", async ({ page, pacs }) => {
  pacs.on("GET", RT_FILE, () => ({ status: 500, json: { Message: "the file is missing from storage" } }));
  await open(page);
  await lay(page, /Pre-Dose, RIGHT/);
  await expect(page.getByText("The segmentation could not be shown")).toBeVisible();
  await expect(page.getByText("the file is missing from storage")).toBeVisible();
});

test("a study's PDF is read from the series list, and never hung in a viewport", async ({ page, pacs }) => {
  // "All series" hangs every series of the study, one a viewport: the document, the SEG and
  // the RTSTRUCT were hung too, each a viewport saying it has no images
  // the study's series as recorded, with a document numbered before them all
  const STUDY = "9fe34720-0aef8f74-788ae8a7-74ce28b1-45495d68";
  const har = JSON.parse(readFileSync(HAR, "utf8"));
  const entry = har.log.entries.find((e: any) => e.request.url.endsWith(`/studies/${STUDY}/series`));
  const doc = { ID: "doc-series", ParentStudy: STUDY, Instances: ["doc-instance"], MainDicomTags: { Modality: "DOC", SeriesNumber: "0", SeriesDescription: "KiTS19 data paper" } };
  await page.route(`**/studies/${STUDY}/series`, (route) => route.fulfill({ json: [doc, ...JSON.parse(entry.response.content.text)] }));
  const pdf = readFileSync(path.resolve(import.meta.dirname, "../../crates/pacs-core/tests/data/documents/kits19.pdf"));
  pacs.on("GET", "/instances/doc-instance/pdf", () => ({ body: pdf as any, headers: { "content-type": "application/pdf" } }));
  await page.goto(`#/view/${STUDY}`);
  // the CT is hung, not the document numbered first
  await expect(page.locator(".ov-tl").first()).toContainText("Recon 2: +C");
  await page.getByRole("button", { name: /KiTS19 data paper/ }).click();
  await expect(page.getByRole("dialog", { name: "KiTS19 data paper" }).locator("iframe")).toHaveAttribute("src", "http://pacs.test/instances/doc-instance/pdf");
  // the viewport still shows the CT
  await expect(page.locator(".ov-tl").first()).toContainText("Recon 2: +C");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Hanging protocols" }).click();
  await page.getByRole("menuitem", { name: /All series/ }).click();
  await expect(page.locator(".ov-tl").first()).toContainText("Recon 2: +C");
  await expect(page.locator(".cell")).toHaveCount(1);
  await expect(page.getByText("This series has no images")).toHaveCount(0);
});
