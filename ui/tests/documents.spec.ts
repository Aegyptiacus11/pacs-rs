import { readFileSync } from "node:fs";
import path from "node:path";
import { test, expect, fixture } from "./pacs";

// a real PDF (CC0), as pacs-core's tests use it
const PDF = readFileSync(path.resolve(import.meta.dirname, "../../crates/pacs-core/tests/data/documents/kits19.pdf"));
const LITS = "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb";
const DOC_SERIES = { ID: "doc-series", Instances: ["doc-instance"], MainDicomTags: { Modality: "DOC", SeriesDescription: "KiTS19 data paper", SeriesNumber: "2" } };
const READER = { id: "r1", login: "reader", role: "reader", permissions: ["read"], projects: null };

type Page = import("@playwright/test").Page;

test.beforeEach(({ pacs }) => {
  pacs.on("GET", /^\/studies\/[\w-]+\/statistics$/, () => ({ json: { DiskSize: "0" } }));
  pacs.on("GET", /^\/studies\/[\w-]+\/series$/, () => ({ json: [] }));
  pacs.on("GET", /^\/series\/[\w-]+\/instances$/, () => ({ json: [] }));
  // a study opened by its link is asked for before the list is in
  pacs.on("GET", /^\/studies\/[\w-]+$/, (_, url) => ({ json: fixture("studies.json").Studies.find((s: any) => url.pathname.endsWith(s.ID)) }));
});

/** The study's ⋯ menu, open. */
async function menu(page: Page) {
  await page.goto(`#/studies?open=${LITS}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await pane.getByRole("button", { name: "More actions" }).click();
}

test("a PDF chosen from the study's menu is sent as it is, titled by its name", async ({ page, pacs }) => {
  let sent: { body: Buffer | null; title: string | null; type: string | undefined } | null = null;
  pacs.on("POST", `/studies/${LITS}/documents`, (route, url) => {
    sent = { body: route.request().postDataBuffer(), title: url.searchParams.get("title"), type: route.request().headers()["content-type"] };
    return { json: { ID: "doc-instance", ParentSeries: "doc-series", Status: "Success" } };
  });
  await menu(page);
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("menuitem", { name: "Attach PDF…" }).click();
  await (await chooser).setFiles({ name: "KiTS19 data paper.pdf", mimeType: "application/pdf", buffer: PDF });
  await expect(page.getByText("PDF attached")).toBeVisible();
  expect(sent!.title).toBe("KiTS19 data paper");
  expect(sent!.type).toBe("application/pdf");
  expect(Buffer.compare(sent!.body!, PDF)).toBe(0);
});

test("a file that is not a PDF is refused before anything is sent", async ({ page, pacs }) => {
  let asked = false;
  pacs.on("POST", `/studies/${LITS}/documents`, () => ((asked = true), { json: {} }));
  await menu(page);
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("menuitem", { name: "Attach PDF…" }).click();
  // a real DICOM file given a .pdf name
  const dicom = readFileSync(path.resolve(import.meta.dirname, "../../docs/sample/00001.dcm"));
  await (await chooser).setFiles({ name: "report.pdf", mimeType: "application/pdf", buffer: dicom });
  await expect(page.getByText("report.pdf is not a PDF")).toBeVisible();
  expect(asked).toBe(false);
});

test("a PDF the server refuses says why", async ({ page, pacs }) => {
  pacs.on("POST", `/studies/${LITS}/documents`, () => ({ status: 409, json: { Message: "the study came in through project liver" } }));
  await menu(page);
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("menuitem", { name: "Attach PDF…" }).click();
  await (await chooser).setFiles({ name: "a.pdf", mimeType: "application/pdf", buffer: PDF });
  await expect(page.getByText("Not attached")).toBeVisible();
  await expect(page.getByText("the study came in through project liver")).toBeVisible();
});

test("a role that may only read attaches nothing", async ({ page, pacs }) => {
  const payload = Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 3600 })).toString("base64url");
  pacs.on("GET", "/ui/config.json", () => ({ json: { ...fixture("config.json"), auth: true } }));
  pacs.on("POST", "/auth/refresh", () => ({ json: { access_token: `e30.${payload}.sig`, token_type: "Bearer" } }));
  pacs.on("GET", "/auth/me", () => ({ json: READER }));
  pacs.on("GET", "/roles", () => ({ json: [{ id: "reader", name: "reader", permissions: ["read"], locked: false }] }));
  await menu(page);
  await expect(page.getByRole("menuitem", { name: "Copy Study UID" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Attach PDF…" })).toHaveCount(0);
});

test("a study's documents are listed apart from its series, and one opens in the PDF viewer", async ({ page, pacs }) => {
  pacs.on("GET", `/studies/${LITS}/series`, () => ({ json: [DOC_SERIES] }));
  let read = false;
  pacs.on("GET", "/instances/doc-instance/pdf", () => ((read = true), { body: PDF as any, headers: { "content-type": "application/pdf" } }));
  await page.goto(`#/studies?open=${LITS}`);
  const pane = page.getByRole("complementary", { name: "The chosen study" });
  await pane.getByRole("button", { name: /KiTS19 data paper/ }).click();
  const dialog = page.getByRole("dialog", { name: "KiTS19 data paper" });
  // the frame is the browser's own PDF viewer, on the server's /pdf (no login here: no token)
  await expect(dialog.locator("iframe")).toHaveAttribute("src", "http://pacs.test/instances/doc-instance/pdf");
  await expect(dialog.getByRole("link", { name: "Download" })).toHaveAttribute("href", "http://pacs.test/instances/doc-instance/pdf");
  await expect.poll(() => read).toBe(true);
  // not among the image series
  await expect(pane.getByRole("button", { name: "Open this series" })).toHaveCount(0);
});
