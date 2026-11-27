import { test, expect, fixture } from "./pacs";

// a server with one remote AE of PACS_DIMSE_REMOTES (hospital) and one added in the settings
// (self), and its refusals, as a pacs-server answered them (remote-*.json)
const ADMIN = { login: "admin", role: "admin", permissions: ["read", "write", "remote", "delete", "users"] };
type Page = import("@playwright/test").Page;

test.beforeEach(({ pacs }) => {
  pacs.on("GET", "/modalities", () => ({ json: fixture("remote-modalities-managed.json") }));
});

const rows = (page: Page) => page.getByRole("row");
const dialog = (page: Page) => page.getByRole("dialog");

test("the remote archives are listed, those of the settings locked", async ({ page }) => {
  await page.goto("#/settings/remotes");
  await expect(rows(page).filter({ hasText: "HOSP_PACS" })).toContainText("10.0.0.5:104");
  await expect(rows(page).filter({ hasText: "HOSP_PACS" }).getByLabel("Set in the server's settings")).toBeVisible();
  await expect(rows(page).filter({ hasText: "HOSP_PACS" }).getByRole("button", { name: /^Actions on/ })).toHaveCount(0);
  await expect(rows(page).filter({ hasText: "127.0.0.1:11193" }).getByRole("button", { name: "Actions on self" })).toBeVisible();
});

test("an archive is added: checked here first, then sent", async ({ page, pacs }) => {
  let body: any = null;
  pacs.on("PUT", "/modalities/ws-2", (route) => {
    body = route.request().postDataJSON();
    return { json: { AET: "WS2", Host: "ws2.hospital.local", Port: 11112, TLS: false, MayReceive: true, Fixed: false } };
  });
  await page.goto("#/settings/remotes");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  // nothing sent while a value cannot be right
  await dialog(page).getByLabel("Name").fill("ws 2");
  await dialog(page).getByLabel("AE title").fill("A_TITLE_FAR_TOO_LONG");
  await dialog(page).getByLabel("Port").fill("70000");
  await dialog(page).getByRole("button", { name: "Add" }).click();
  await expect(dialog(page)).toContainText("1 to 64 letters, digits, - and _");
  await expect(dialog(page)).toContainText("At most 16 characters");
  await expect(dialog(page)).toContainText("1 to 65535");
  await expect(dialog(page)).toContainText("Required");
  expect(body).toBeNull();

  await dialog(page).getByLabel("Name").fill("ws-2");
  await dialog(page).getByLabel("AE title").fill("WS2");
  await dialog(page).getByLabel("Host").fill("ws2.hospital.local");
  await dialog(page).getByLabel("Port").fill("11112");
  await dialog(page).getByRole("checkbox", { name: "Studies may be sent to it" }).click();
  await dialog(page).getByRole("button", { name: "Add" }).click();
  await expect(page.getByText("ws-2 added")).toBeVisible();
  expect(body).toEqual({ AET: "WS2", Host: "ws2.hospital.local", Port: 11112, TLS: false, MayReceive: true });
  await expect(dialog(page)).toHaveCount(0);
});

test("an archive the server refuses says why, and the dialog stays", async ({ page, pacs }) => {
  pacs.on("PUT", "/modalities/ws", () => ({ status: 400, json: fixture("remote-put-refused.json") }));
  await page.goto("#/settings/remotes");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await dialog(page).getByLabel("Name").fill("ws");
  await dialog(page).getByLabel("AE title").fill("WS");
  await dialog(page).getByLabel("Host").fill("10.0.0.9");
  await dialog(page).getByRole("checkbox", { name: "Reached over TLS" }).click();
  await dialog(page).getByRole("button", { name: "Add" }).click();
  await expect(page.getByText("The remote archive was not added")).toBeVisible();
  await expect(page.getByText(/no DIMSE TLS to call it with/)).toBeVisible();
  await expect(dialog(page)).toBeVisible();
});

test("an archive added is changed with its values, and removed once confirmed", async ({ page, pacs }) => {
  let body: any = null;
  let removed = false;
  pacs.on("PUT", "/modalities/self", (route) => ((body = route.request().postDataJSON()), { json: {} }));
  pacs.on("DELETE", "/modalities/self", () => ((removed = true), { body: "" }));
  await page.goto("#/settings/remotes");
  await page.getByRole("button", { name: "Actions on self" }).click();
  await page.getByRole("menuitem", { name: "Change…" }).click();
  await expect(dialog(page).getByLabel("Name")).toHaveValue("self");
  await expect(dialog(page).getByLabel("Name")).toBeDisabled();
  await expect(dialog(page).getByLabel("Port")).toHaveValue("11193");
  await dialog(page).getByLabel("Port").fill("11194");
  await dialog(page).getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("self changed")).toBeVisible();
  expect(body).toEqual({ AET: "PACSRS", Host: "127.0.0.1", Port: 11194, TLS: false, MayReceive: false });

  await page.getByRole("button", { name: "Actions on self" }).click();
  await page.getByRole("menuitem", { name: "Remove…" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("Its studies already here stay");
  await page.getByRole("alertdialog").getByRole("button", { name: "Remove" }).click();
  await expect(page.getByText("self removed")).toBeVisible();
  expect(removed).toBe(true);
});

test("a removal the server refuses says why", async ({ page, pacs }) => {
  pacs.on("DELETE", "/modalities/self", () => ({ status: 409, json: { ...fixture("remote-delete-refused.json"), Details: "self is what PACS_FORWARD sends to" } }));
  await page.goto("#/settings/remotes");
  await page.getByRole("button", { name: "Actions on self" }).click();
  await page.getByRole("menuitem", { name: "Remove…" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Remove" }).click();
  await expect(page.getByText("self was not removed")).toBeVisible();
  await expect(page.getByText(/what PACS_FORWARD sends to/)).toBeVisible();
});

test("Test runs a C-ECHO and says how it went", async ({ page, pacs }) => {
  pacs.on("POST", "/modalities/self/echo", () => ({ json: fixture("remote-echo.json") }));
  pacs.on("POST", "/modalities/hospital/echo", () => ({ status: 502, json: fixture("remote-echo-failed.json") }));
  await page.goto("#/settings/remotes");
  await page.getByRole("button", { name: "Test the link to self" }).click();
  await expect(rows(page).filter({ hasText: "PACSRS" })).toContainText(/Answered in \d+ ms/);
  await page.getByRole("button", { name: "Test the link to hospital" }).click();
  await expect(rows(page).filter({ hasText: "HOSP_PACS" })).toContainText(fixture("remote-echo-failed.json").Message);
});

test("only an administrator manages them; the Remote page leads there", async ({ page, pacs }) => {
  pacs.signedIn({ login: "radiologist", role: "radiologist", permissions: ["read", "remote"] });
  pacs.on("POST", /^\/modalities\/[\w-]+\/echo$/, () => ({ json: fixture("remote-echo.json") }));
  await page.goto("#/remote");
  await expect(page.getByRole("heading", { name: "Remote archives" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Manage the archives" })).toHaveCount(0);
  await page.goto("#/settings");
  await expect(page.getByRole("link", { name: "Remote archives" })).toHaveCount(0);
});

test("an administrator finds the management from the Remote page", async ({ page, pacs }) => {
  pacs.signedIn(ADMIN);
  pacs.on("POST", /^\/modalities\/[\w-]+\/echo$/, () => ({ json: fixture("remote-echo.json") }));
  await page.goto("#/remote");
  await page.getByRole("link", { name: "Manage the archives" }).click();
  await expect(page).toHaveURL(/#\/settings\/remotes$/);
  await expect(rows(page).filter({ hasText: "HOSP_PACS" })).toBeVisible();
});
