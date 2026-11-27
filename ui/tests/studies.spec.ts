import { test, expect } from "./pacs";

test("the study list shows the archive's studies", async ({ page, pacs }) => {
  await page.goto("#/studies");
  await expect(page.getByRole("heading", { name: "Studies", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" })).toHaveCount(1);
  await expect(page.getByText("3 studies", { exact: true })).toBeVisible();
  expect(pacs.asked).toContain("GET /ui/api/studies?sort=date&order=desc&limit=50");
});


test("a server that does not answer is shown in the top bar", async ({ page, pacs }) => {
  pacs.on("GET", "/healthz", () => ({ status: 503, body: "not ready" }));
  await page.goto("#/studies");
  await expect(page.getByRole("banner")).toContainText("Offline");
});
