import path from "node:path";
import { test, expect } from "./pacs";

// tests/data/prostate-dce, as the viewer asked a pacs-server for it (record-viewer.mjs)
const HAR = path.join(import.meta.dirname, "fixtures/viewer-dce.har");
const VIEW = "#/view/f75ba115-80e6b64d-1eff1fe8-8e336e6b-895fbe3c/24328825-bd2698af-4128076a-42bea43c-bca04b77";

test.beforeEach(async ({ page, pacs }) => {
  // the series' requests from the recording; anything else goes to the stand-in, which fails
  // the test for a request nobody answers
  void pacs;
  await page.routeFromHAR(HAR, { url: /^http:\/\/pacs\.test\/(?!ui\/(?!config\.json|api\/))/, notFound: "fallback" });
});

/** The lines drawn on each viewport's annotation layer. */
const lines = (page: import("@playwright/test").Page) =>
  page.locator(".cs").evaluateAll((cells) => cells.map((c) => c.querySelectorAll("svg line").length));

test("an image that names VOI LUT Function LINEAR is drawn with its own window", async ({ page }) => {
  await page.goto(VIEW);
  // slice 6 of 10, opened in the middle: its Window Center 1070 and Width 1860
  const corner = page.locator(".ov-bl").first();
  await expect(corner).toContainText("Image 6 / 10");
  await expect(corner).toContainText("W 1860  L 1070");
});

test("the active viewport's plane is drawn on the others, and G takes it away", async ({ page }) => {
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText("W 1860");
  // a sagittal plane beside the axial images; the new view is the active one
  await page.keyboard.press("n");
  await expect(page.locator(".ov-tl", { hasText: "Sagittal" })).toBeVisible();
  const toggle = page.getByRole("button", { name: "Lines" });
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  // the sagittal plane cuts the axial image along one line
  await expect.poll(async () => (await lines(page))[0]).toBeGreaterThan(0);

  await page.keyboard.press("g");
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await lines(page))[0]).toBe(0);

  await toggle.click();
  await expect.poll(async () => (await lines(page))[0]).toBeGreaterThan(0);
});

test("one viewport has no reference lines to show, and no Lines button", async ({ page }) => {
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText("W 1860");
  await expect(page.getByRole("button", { name: "Lines" })).toHaveCount(0);
});

test("the measurements are one button that takes up the last one chosen", async ({ page }) => {
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText("W 1860");
  // one button, Length on a first visit, and none of the others on the bar
  await expect(page.getByRole("button", { name: "Length", exact: true })).toHaveAttribute("aria-pressed", "false");
  for (const name of ["Angle", "Cobb", "Bidir.", "Ellipse", "Rectangle", "Value"]) await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);

  // a key chooses one: the button shows it, pressed
  await page.keyboard.press("a");
  await expect(page.getByRole("button", { name: "Angle", exact: true })).toHaveAttribute("aria-pressed", "true");

  // the menu has them all; one chosen there takes the button
  await page.getByRole("button", { name: "Measurements" }).click();
  for (const name of ["Length", "Angle", "Cobb angle", "Bidirectional", "Ellipse ROI", "Rectangle ROI", "Pixel value"])
    await expect(page.getByRole("menuitem", { name: new RegExp(`^${name}`) })).toBeVisible();
  await page.getByRole("menuitem", { name: /^Bidirectional/ }).click();
  await expect(page.getByRole("button", { name: "Bidir.", exact: true })).toHaveAttribute("aria-pressed", "true");

  // another tool lets it go, and the button keeps the last measurement for next time
  await page.keyboard.press("p");
  await expect(page.getByRole("button", { name: "Bidir.", exact: true })).toHaveAttribute("aria-pressed", "false");
  await page.reload();
  await expect(page.getByRole("button", { name: "Bidir.", exact: true })).toBeVisible();
});

test("the measurement button works when the browser refuses storage", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "localStorage", { get: () => { throw new DOMException("refused", "SecurityError"); } });
  });
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText("W 1860");
  await page.keyboard.press("c");
  await expect(page.getByRole("button", { name: "Cobb", exact: true })).toHaveAttribute("aria-pressed", "true");
});
