import { test, expect, fixture } from "./pacs";

const LITS = "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb";
const MR1 = "7b5f82d7-011e7118-ffac48a8-9204a296-775e6f54";
const READER = { id: "r1", login: "reader", role: "reader", permissions: ["read"], projects: null };

// the pane's series are not what these tests are about
test.beforeEach(({ pacs }) => {
  pacs.on("GET", /^\/studies\/[\w-]+\/series$/, () => ({ json: [] }));
  pacs.on("GET", /^\/studies\/[\w-]+\/statistics$/, () => ({ json: { DiskSize: "0" } }));
  pacs.on("GET", "/tools/labels", () => ({ json: fixture("labels.json") }));
  // a study opened by its link is asked for before the list is in
  pacs.on("GET", /^\/studies\/[\w-]+$/, (_, url) => ({ json: fixture("studies.json").Studies.find((s: any) => url.pathname.endsWith(s.ID)) }));
});

const row = (page: import("@playwright/test").Page, text: string) => page.getByRole("row").filter({ hasText: text });

test("a study's labels show on its row", async ({ page }) => {
  await page.goto("#/studies");
  await expect(row(page, "Three slices of LiTS volume 0").getByLabel("Labels: follow-up, teaching")).toBeVisible();
  await expect(row(page, "e+1").getByLabel("Labels: teaching")).toBeVisible();
  await expect(row(page, "CompressedSamples^MR1").or(row(page, "Mr1")).getByLabel(/^Labels:/)).toHaveCount(0);
});

test("the Label filter asks the server for the studies with the label", async ({ page, pacs }) => {
  pacs.on("GET", "/ui/api/studies", (_, url) =>
    url.searchParams.get("summary") === "true"
      ? { json: fixture("studies-summary.json") }
      : { json: fixture(url.searchParams.get("label") === "follow-up" ? "studies-follow-up.json" : "studies.json") },
  );
  await page.goto("#/studies");
  await page.getByRole("button", { name: "Label", exact: true }).click();
  // each label with how many studies have it
  await expect(page.getByRole("option", { name: /teaching\s*2/ })).toBeVisible();
  await page.getByRole("option", { name: /follow-up/ }).click();
  await page.keyboard.press("Escape");
  await expect.poll(() => pacs.asked).toContain("GET /ui/api/studies?label=follow-up&sort=date&order=desc&limit=50");
  await expect(page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" })).toHaveCount(1);
  await expect(row(page, "e+1")).toHaveCount(0);
  // Reset takes it off
  await page.getByRole("button", { name: "Reset" }).click();
  await expect(row(page, "e+1")).toHaveCount(1);
});

test("no study has a label: no Label filter", async ({ page, pacs }) => {
  pacs.on("GET", "/ui/api/studies/facets", () => ({ json: { ...fixture("studies-facets.json"), Labels: {} } }));
  await page.goto("#/studies");
  await expect(page.getByRole("button", { name: "Modality", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Label", exact: true })).toHaveCount(0);
});

test("a label is put on a study and taken off from its pane", async ({ page, pacs }) => {
  const asked: string[] = [];
  pacs.on("PUT", /^\/studies\/[\w-]+\/labels\/[\w-]+$/, (route) => {
    asked.push(`PUT ${new URL(route.request().url()).pathname}`);
    return { body: "" };
  });
  pacs.on("DELETE", /^\/studies\/[\w-]+\/labels\/[\w-]+$/, (route) => {
    asked.push(`DELETE ${new URL(route.request().url()).pathname}`);
    return { body: "" };
  });
  await page.goto(`#/studies?open=${MR1}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await pane.getByRole("button", { name: "Add label" }).click();
  // the labels in use are offered; a new one is typed
  await expect(page.getByRole("option", { name: "teaching" })).toBeVisible();
  await page.getByRole("combobox", { name: "Label" }).fill("cohort-a");
  await page.getByRole("option", { name: "Create “cohort-a”" }).click();
  await expect(pane.getByRole("button", { name: "Remove the label cohort-a" })).toBeVisible();
  await expect.poll(() => asked).toEqual([`PUT /studies/${MR1}/labels/cohort-a`]);

  await pane.getByRole("button", { name: "Remove the label cohort-a" }).click();
  await expect(pane.getByRole("button", { name: "Remove the label cohort-a" })).toHaveCount(0);
  await expect.poll(() => asked).toEqual([`PUT /studies/${MR1}/labels/cohort-a`, `DELETE /studies/${MR1}/labels/cohort-a`]);
});

test("a label that is not one is refused before it is sent", async ({ page }) => {
  await page.goto(`#/studies?open=${MR1}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await pane.getByRole("button", { name: "Add label" }).click();
  await page.getByRole("combobox", { name: "Label" }).fill("two words");
  await expect(page.getByRole("alert")).toHaveText("Letters, digits, - and _ only");
  await expect(page.getByRole("option", { name: /Create/ })).toHaveCount(0);
});

test("a label the server refuses comes off again and says why", async ({ page, pacs }) => {
  pacs.on("PUT", /^\/studies\/[\w-]+\/labels\/[\w-]+$/, () => ({ status: 403, json: { Message: "Forbidden" } }));
  await page.goto(`#/studies?open=${MR1}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await pane.getByRole("button", { name: "Add label" }).click();
  await page.getByRole("option", { name: "teaching" }).click();
  await expect(page.getByText("“teaching” was not put on")).toBeVisible();
  await expect(pane.getByRole("button", { name: "Remove the label teaching" })).toHaveCount(0);
});

test("one label goes on every study ticked", async ({ page, pacs }) => {
  const put: string[] = [];
  pacs.on("PUT", /^\/studies\/[\w-]+\/labels\/[\w-]+$/, (route) => {
    put.push(new URL(route.request().url()).pathname);
    return { body: "" };
  });
  await page.goto("#/studies");
  for (const text of ["Three slices of LiTS volume 0", "e+1"]) await row(page, text).getByRole("checkbox").check();
  await page.getByRole("button", { name: "Label", exact: true }).last().click();
  await page.getByRole("combobox", { name: "Label" }).fill("cohort-b");
  await page.getByRole("option", { name: "Create “cohort-b”" }).click();
  await expect(page.getByText("“cohort-b” put on 2 studies")).toBeVisible();
  expect(put.sort()).toEqual([`/studies/${LITS}/labels/cohort-b`, "/studies/8a8cf898-ca27c490-d0c7058c-929d0581-2bbf104d/labels/cohort-b"].sort());
});

test("a role that may only read sees the labels but cannot change them", async ({ page, pacs }) => {
  // signed in, as a role with `read` only (a token whose payload says it runs for an hour)
  const payload = Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 3600 })).toString("base64url");
  pacs.on("GET", "/ui/config.json", () => ({ json: { ...fixture("config.json"), auth: true } }));
  pacs.on("POST", "/auth/refresh", () => ({ json: { access_token: `e30.${payload}.sig`, token_type: "Bearer" } }));
  pacs.on("GET", "/auth/me", () => ({ json: READER }));
  pacs.on("GET", "/roles", () => ({ json: [{ id: "reader", name: "reader", permissions: ["read"], locked: false }] }));
  await page.goto(`#/studies?open=${LITS}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await expect(pane.getByText("follow-up")).toBeVisible();
  await expect(pane.getByRole("button", { name: "Add label" })).toHaveCount(0);
  await expect(pane.getByRole("button", { name: /Remove the label/ })).toHaveCount(0);
});
