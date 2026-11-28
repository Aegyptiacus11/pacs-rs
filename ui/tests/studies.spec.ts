import { test, expect, fixture } from "./pacs";

test("the study list shows the archive's studies", async ({ page, pacs }) => {
  await page.goto("#/studies");
  await expect(page.getByRole("heading", { name: "Studies", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" })).toHaveCount(1);
  await expect(page.getByText("3 studies", { exact: true })).toBeVisible();
  expect(pacs.asked).toContain("GET /ui/api/studies?sort=date&order=desc&limit=50");
});

test("a failing study list says why", async ({ page, pacs }) => {
  pacs.on("GET", "/ui/api/studies", () => ({ status: 500, json: { Message: "the index cannot be read" } }));
  await page.goto("#/studies");
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("The studies could not be listed");
  await expect(alert).toContainText("the index cannot be read");
  // it asks again when told to, and shows the list once the server answers
  pacs.on("GET", "/ui/api/studies", (_, url) => ({ json: url.searchParams.get("summary") === "true" ? fixture("studies-summary.json") : fixture("studies.json") }));
  await alert.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" })).toHaveCount(1);
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("a server that does not answer is shown in the top bar", async ({ page, pacs }) => {
  pacs.on("GET", "/healthz", () => ({ status: 503, body: "not ready" }));
  await page.goto("#/studies");
  await expect(page.getByRole("banner")).toContainText("Offline");
});

test("sex and age are behind More filters, which stays open while one is used", async ({ page, pacs }) => {
  await page.goto("#/studies");
  await expect(page.getByRole("button", { name: "Modality", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Sex", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Age", exact: true })).toHaveCount(0);

  await page.getByRole("button", { name: "More filters" }).click();
  // the keyboard is not lost with the button: it is on Sex
  await expect(page.getByRole("button", { name: "Sex", exact: true })).toBeFocused();
  // unused, they can be put away again
  await page.getByRole("button", { name: "Fewer filters" }).click();
  await expect(page.getByRole("button", { name: "Sex", exact: true })).toHaveCount(0);

  await page.getByRole("button", { name: "More filters" }).click();
  await page.getByRole("button", { name: "Sex", exact: true }).click();
  await page.getByRole("option", { name: /Female/ }).click();
  await page.keyboard.press("Escape");
  await expect.poll(() => pacs.asked.some((a) => a.startsWith("GET /ui/api/studies?") && a.includes("sex=F"))).toBe(true);
  // in use, they stay: nothing puts them away but Reset
  await expect(page.getByRole("button", { name: "Fewer filters" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "More filters" })).toHaveCount(0);

  // Reset clears them and puts them away again
  await page.getByRole("button", { name: "Reset" }).click();
  await expect(page.getByRole("button", { name: "More filters" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Sex", exact: true })).toHaveCount(0);
});
