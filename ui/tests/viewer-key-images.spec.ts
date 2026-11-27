import { readFileSync } from "node:fs";
import path from "node:path";
import { test, expect, fixture, FIRST_IMAGE } from "./pacs";

// tests/data/kidney-seg as recorded (record-viewer.mjs kidney-seg); the Key Object Selection
// pacs-rs made of its images 5 and 7, as a pacs-server gave it back (kidney-ko-*.json)
const HAR = path.join(import.meta.dirname, "fixtures/viewer-kidney.har");
const STUDY = "9fe34720-0aef8f74-788ae8a7-74ce28b1-45495d68";
const VIEW = `#/view/${STUDY}/23efc5ff-23e7ab69-fb9df9a0-5aca5ab6-0e406fbb`;
type Page = import("@playwright/test").Page;

test.beforeEach(async ({ page, pacs }) => {
  pacs.on("PUT", /^\/ui\/api\/annotations\//, () => ({ status: 204 }));
  await page.routeFromHAR(HAR, { url: /^http:\/\/pacs\.test\/(?!ui\/(?!config\.json|api\/))(?!instances\/[\w-]+\/file$)/, notFound: "fallback" });
});

/** The study's series as recorded, and its Key Object Selection with them. */
async function withSavedKeys(page: Page) {
  const har = JSON.parse(readFileSync(HAR, "utf8"));
  const recorded = JSON.parse(har.log.entries.find((e: any) => e.request.url.endsWith(`/studies/${STUDY}/series`)).response.content.text);
  const ko = fixture("kidney-ko-series.json");
  await page.route(`**/studies/${STUDY}/series`, (route) => route.fulfill({ json: [...recorded, ko] }));
  await page.route(`**/series/${ko.MainDicomTags.SeriesInstanceUID}/metadata`, (route) => route.fulfill({ json: fixture("kidney-ko-metadata.json") }));
}

async function open(page: Page) {
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText(/W \d+ {2}L \d+/, FIRST_IMAGE);
  await page.locator(".cell").first().focus();
}
const corner = (page: Page) => page.locator(".ov-tl").first();
const panel = (page: Page) => page.getByRole("region", { name: "Key images" });

test("K marks images, Save makes them a Key Object Selection of the study", async ({ page, pacs }) => {
  let body: any = null;
  pacs.on("POST", `/studies/${STUDY}/key-images`, (route) => ((body = route.request().postDataJSON()), { json: { ID: "ko", Status: "Success" } }));
  await open(page);
  await expect(page.locator(".ov-bl").first()).toContainText("Image 5 / 8");
  await page.keyboard.press("k");
  await expect(corner(page)).toContainText("★ Key image, not saved");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("k");
  await expect(panel(page).getByRole("listitem")).toHaveText(["CT 4 Recon 2: +C · image 5", "CT 4 Recon 2: +C · image 7"]);
  // a second K on a mark not saved takes it off
  await page.keyboard.press("k");
  await expect(panel(page).getByRole("listitem")).toHaveCount(1);
  await page.keyboard.press("k");
  await panel(page).getByLabel("Key images description").fill("Right renal mass");
  await panel(page).getByRole("button", { name: "Save 2 key images" }).click();
  await expect(page.getByText("2 key images saved")).toBeVisible();
  expect(body.Description).toBe("Right renal mass");
  expect(body.Images).toHaveLength(2);
  expect(body.Images.every((i: unknown) => typeof i === "string")).toBe(true);
  await expect(corner(page)).toContainText("★ Key image");
  await expect(corner(page)).not.toContainText("not saved");
  await expect(panel(page).getByRole("button", { name: /^Save/ })).toHaveCount(0);
});

test("saved key images are read from the study's Key Object Selection, and shown with a click", async ({ page }) => {
  await withSavedKeys(page);
  await open(page);
  // image 5 is one of them, from the study's KO
  await expect(corner(page)).toContainText("★ Key image");
  await page.getByRole("button", { name: "2 key" }).click();
  const list = panel(page).getByRole("listitem");
  await expect(list).toHaveCount(2);
  await expect(list.getByText("saved")).toHaveCount(2);
  await list.nth(1).getByRole("button").click();
  await expect(page.locator(".ov-bl").first()).toContainText("Image 7 / 8");
  // a saved one stays: K says how it goes
  await page.locator(".cell").first().focus();
  await page.keyboard.press("k");
  await expect(page.getByText(/delete its key image series/)).toBeVisible();
  await expect(list).toHaveCount(2);
});

test("the key image series in the series list opens the key images", async ({ page }) => {
  await withSavedKeys(page);
  await open(page);
  await page.getByRole("button", { name: /Key images/ }).filter({ hasText: "KO" }).click();
  await expect(panel(page)).toBeVisible();
  // the viewport still shows the CT
  await expect(page.locator(".ov-tl").first()).toContainText("Recon 2: +C");
});

test("key images the server refuses stay marked, not saved, and it says why", async ({ page, pacs }) => {
  pacs.on("POST", `/studies/${STUDY}/key-images`, () => ({ status: 403, json: { Message: "Limited to projects" } }));
  await open(page);
  await page.keyboard.press("k");
  await panel(page).getByRole("button", { name: "Save 1 key image" }).click();
  await expect(page.getByText("The key images were not saved")).toBeVisible();
  await expect(page.getByText("Limited to projects")).toBeVisible();
  await expect(corner(page)).toContainText("not saved");
});

test("an arrow is drawn with a label asked in the page, and kept with the measurements", async ({ page }) => {
  // the recording has a save of its own: this one, registered after it, comes first
  const saved: any[][] = [];
  await page.route("**/ui/api/annotations/**", (route) =>
    route.request().method() === "PUT" ? (saved.push(route.request().postDataJSON()), route.fulfill({ status: 204 })) : route.fallback(),
  );
  await open(page);
  await page.keyboard.press("Shift+A");
  const box = (await page.locator(".cs").first().boundingBox())!;
  const drag = async () => {
    await page.mouse.move(box.x + box.width * 0.4, box.y + box.height * 0.4);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.55, box.y + box.height * 0.6, { steps: 8 });
    await page.mouse.up();
  };
  // Cancel: no arrow
  await drag();
  await page.getByRole("dialog", { name: "Arrow label" }).getByRole("button", { name: "Cancel" }).click();
  await page.waitForTimeout(300);
  expect(saved.flat().filter((a) => a.metadata.toolName === "ArrowAnnotate")).toHaveLength(0);
  await drag();
  const dialog = page.getByRole("dialog", { name: "Arrow label" });
  await dialog.getByLabel("Arrow label").fill("Mass");
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect.poll(() => saved.at(-1)?.filter((a: any) => a.metadata.toolName === "ArrowAnnotate").map((a: any) => a.data.label)).toEqual(["Mass"]);
  await expect(page.locator(".cs svg text", { hasText: "Mass" })).toHaveCount(1);
});

test("a key image is marked on an image as acquired, not in MPR", async ({ page }) => {
  await open(page);
  await page.keyboard.press("x");
  await expect(page.locator(".ov-tl", { hasText: "Sagittal" })).toBeVisible();
  await page.locator(".cell").first().focus();
  await page.keyboard.press("k");
  await expect(page.getByText("Key images are marked on an image as acquired")).toBeVisible();
});
