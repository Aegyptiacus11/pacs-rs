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

test("a slow machine is not taken for one whose WebGL draws blank", async ({ page }) => {
  // On a loaded machine the first image was drawn later than the check's fixed wait of 600 ms:
  // the viewer judged WebGL blank, switched to reading it back and reloaded, mid-test (the
  // parallel suite, 27 November 2026). Rendering a second late stands for that: Cornerstone
  // renders on animation frames.
  await page.addInitScript(() => {
    const frame = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (callback) => frame(() => setTimeout(() => frame(callback), 1000)) as any;
  });
  await page.goto(VIEW);
  await expect(page.locator(".ov-bl").first()).toContainText("W 1860", { timeout: 20_000 });
  await page.waitForTimeout(3000);
  expect(await page.evaluate(() => localStorage.getItem("pacs-rs-rendering-mode"))).toBeNull();
  await expect(page.getByText(/didn't show the WebGL image|draws blank images/)).toHaveCount(0);
});

test("a browser whose WebGL image never reaches the page is noticed, and read back from then on", async ({ page }) => {
  // as Chrome with ANGLE on Vulkan and Mesa RADV did: WebGL draws, the copy to the page shows nothing
  await page.addInitScript(() => {
    const draw = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (this: CanvasRenderingContext2D, image: any, ...rest: any[]) {
      if (image instanceof HTMLCanvasElement && (image as any).__webgl) return;
      return (draw as any).call(this, image, ...rest);
    } as any;
    const get = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, ...rest: any[]) {
      if (type.startsWith("webgl")) (this as any).__webgl = true;
      return (get as any).call(this, type, ...rest);
    } as any;
  });
  await page.goto(VIEW);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("pacs-rs-rendering-mode")), { timeout: 15_000 }).toBe("readback");
});
