import { test, expect, fixture } from "./pacs";

const LITS = "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055";
const MR1 = "23755877-c2ffb60d-d0df4093-e1f071a3-68b19506";
const ADMIN = { login: "admin", role: "admin", permissions: ["read", "write", "remote", "delete", "modify", "users"] };

/** The record of patient LITS-0 open, its actions menu open. */
async function actions(page: import("@playwright/test").Page) {
  await page.goto(`#/patients?open=${LITS}`);
  await expect(page.getByRole("heading", { name: "Lits Volume 0" })).toBeVisible();
  await page.getByRole("button", { name: "More actions" }).click();
}

test("a patient is modified: only what changed is sent, and Undo restores the trash entry", async ({ page, pacs }) => {
  pacs.signedIn(ADMIN);
  let body: any = null;
  pacs.on("POST", `/patients/${LITS}/modify`, (route) => {
    body = route.request().postDataJSON();
    return { json: { ID: "new-patient-id", Path: "/patients/new-patient-id", Instances: 3, Trash: "7" } };
  });
  let restored = false;
  pacs.on("POST", "/trash/7/restore", () => {
    restored = true;
    return { json: { restored: 3, kept: [] } };
  });

  await actions(page);
  await page.getByRole("menuitem", { name: "Modify patient…" }).click();
  const dialog = page.getByRole("dialog", { name: "Modify patient" });
  await expect(dialog.getByLabel("Family name")).toHaveValue("VOLUME 0");
  await expect(dialog.getByLabel("Patient ID")).toHaveValue("LITS-0");
  await expect(dialog.getByRole("button", { name: "No change" })).toBeDisabled();

  await dialog.getByLabel("Patient ID").fill("H-0042");
  await dialog.getByLabel("Sex").selectOption("F");
  await expect(dialog.getByText("A new ID changes the patient's links.")).toBeVisible();
  await dialog.getByRole("button", { name: "Write 2 changes" }).click();

  expect(body).toEqual({ PatientID: "H-0042", PatientSex: "F" });
  await expect(page.getByText("Patient modified")).toBeVisible();
  await expect(page).toHaveURL(/#\/patients\?open=new-patient-id/);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByText("Undone")).toBeVisible();
  expect(restored).toBe(true);
});

test("a modification the server refuses says why and changes nothing", async ({ page, pacs }) => {
  pacs.signedIn(ADMIN);
  pacs.on("POST", `/patients/${LITS}/modify`, () => ({
    status: 409,
    json: { Message: "Not modified", Details: "study 2.25.3100000 is a de-identified copy in project liver" },
  }));
  await actions(page);
  await page.getByRole("menuitem", { name: "Modify patient…" }).click();
  const dialog = page.getByRole("dialog", { name: "Modify patient" });
  await dialog.getByLabel("Given name").fill("Amina");
  await dialog.getByRole("button", { name: "Write 1 change" }).click();
  await expect(page.getByText("The patient was not modified")).toBeVisible();
  await expect(page.getByText("Not modified: study 2.25.3100000 is a de-identified copy in project liver")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`open=${LITS}`));
});

test("a patient is merged into one found by name", async ({ page, pacs }) => {
  pacs.signedIn(ADMIN);
  let body: any = null;
  pacs.on("POST", `/patients/${LITS}/merge`, (route) => {
    body = route.request().postDataJSON();
    return { json: { ID: MR1, Path: `/patients/${MR1}`, Instances: 3, Trash: "8" } };
  });
  pacs.on("GET", "/ui/api/patients", (_, url) =>
    url.searchParams.get("q") ? { json: { Total: 2, Patients: fixture("patients.json").Patients.slice(0, 2) } } : { json: fixture("patients.json") },
  );
  await actions(page);
  await page.getByRole("menuitem", { name: "Merge into another…" }).click();
  const dialog = page.getByRole("dialog", { name: "Merge into another patient" });
  await dialog.getByLabel("The patient to keep").fill("compressed");
  // the patient merged is not offered as their own target
  await expect(dialog.getByRole("list", { name: "Patients" }).getByRole("button")).toHaveCount(1);
  await dialog.getByRole("button", { name: /Compressedsamples Mr1/i }).click();
  await dialog.getByRole("button", { name: /^Merge into/ }).click();
  expect(body).toEqual({ Into: MR1 });
  await expect(page.getByText(/Merged into/)).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`open=${MR1}`));
});

test("a role without modify has no modification in the menu", async ({ page, pacs }) => {
  pacs.signedIn({ login: "manager", role: "manager", permissions: ["read", "write", "remote", "delete"] });
  await actions(page);
  await expect(page.getByRole("menuitem", { name: "Delete patient…" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Modify patient…" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Merge into another…" })).toHaveCount(0);
});
