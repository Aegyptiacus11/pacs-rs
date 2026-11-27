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


test("an image that names VOI LUT Function LINEAR is drawn with its own window", async ({ page }) => {
  await page.goto(VIEW);
  // slice 6 of 10, opened in the middle: its Window Center 1070 and Width 1860
  const corner = page.locator(".ov-bl").first();
  await expect(corner).toContainText("Image 6 / 10");
  await expect(corner).toContainText("W 1860  L 1070");
});


