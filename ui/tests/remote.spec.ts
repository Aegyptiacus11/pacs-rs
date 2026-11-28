import { test, expect, fixture, type Pacs } from "./pacs";

const LITS = "2.25.3100000";

/** The remote routes answered as the two recorded servers answered: "hospital" works, "lab"
 * does not exist. */
function remotes(pacs: Pacs) {
  pacs.on("GET", "/modalities", () => ({ json: fixture("remote-modalities.json") }));
  pacs.on("POST", "/modalities/hospital/echo", () => ({ json: fixture("remote-echo.json") }));
  pacs.on("POST", "/modalities/lab/echo", () => ({ status: 502, json: fixture("remote-echo-failed.json") }));
  pacs.on("GET", "/modalities/hospital/studies", () => ({ json: fixture("remote-studies.json") }));
  pacs.on("GET", "/modalities/lab/studies", () => ({ status: 502, json: fixture("remote-studies-failed.json") }));
}

test("the remote archives are listed, each tested with a C-ECHO", async ({ page, pacs }) => {
  remotes(pacs);
  await page.goto("#/remote");
  const list = page.getByRole("list", { name: "Remote archives" });
  await expect(list.getByRole("button", { name: /hospital/ })).toContainText("HOSPITAL · remote:11112");
  await expect(list.getByRole("button", { name: /hospital/ })).toContainText(/Answers · \d+ ms/);
  await expect(list.getByRole("button", { name: /lab/ })).toContainText("Not reachable");
  await expect(list.getByRole("button", { name: /lab/ }).getByText("Not reachable")).toHaveAttribute(
    "title",
    /C-ECHO to lab failed: no association with LAB@nowhere:11112/,
  );
});

test("a remote archive's studies are searched and the ticked ones pulled", async ({ page, pacs }) => {
  remotes(pacs);
  let body: any = null;
  pacs.on("POST", "/modalities/hospital/move", (route) => {
    body = route.request().postDataJSON();
    return { status: 202, json: fixture("remote-move-started.json") };
  });
  await page.goto("#/remote?open=hospital");
  await page.getByLabel("Patient name").fill("volume");
  await page.getByRole("button", { name: "Search" }).click();
  await expect.poll(() => pacs.asked).toContain("GET /modalities/hospital/studies?PatientName=volume*&limit=200");

  const studies = page.getByRole("table", { name: "Studies of hospital" });
  await expect(studies.getByRole("row")).toHaveCount(4); // the header and three studies
  await expect(page.getByRole("button", { name: "Pull" })).toBeDisabled();
  await studies.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" }).click();
  await page.getByRole("button", { name: "Pull 1 study" }).click();

  await expect(page.getByText("Pulling 1 study from hospital")).toBeVisible();
  // with its Patient ID, which an archive in Patient Root only needs
  expect(body).toEqual({ PatientID: "LITS-0", StudyInstanceUID: [LITS], Asynchronous: true });
});

test("a search the remote archive fails says why", async ({ page, pacs }) => {
  remotes(pacs);
  await page.goto("#/remote?open=lab");
  await page.getByRole("button", { name: "Search" }).click();
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("lab could not be searched");
  await expect(alert).toContainText("C-FIND to lab failed: no association with LAB@nowhere:11112: could not connect to server");
});

test("a pull the server refuses says why", async ({ page, pacs }) => {
  remotes(pacs);
  pacs.on("POST", "/modalities/hospital/move", () => ({
    status: 400,
    json: { Message: "Bad request", Details: "the body names no valid Study Instance UID" },
  }));
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByRole("checkbox", { name: "Tick every study shown" }).click();
  await page.getByRole("button", { name: "Pull 3 studies" }).click();
  await expect(page.getByText("The pull from hospital did not start")).toBeVisible();
  await expect(page.getByText("Bad request: the body names no valid Study Instance UID")).toBeVisible();
});

test("without remote archives the page says how to add one", async ({ page, pacs }) => {
  pacs.on("GET", "/modalities", () => ({ json: {} }));
  await page.goto("#/remote");
  await expect(page.getByText("No remote archive yet")).toBeVisible();
  // added in the settings, by whoever manages them (without a login, anyone)
  await page.getByRole("link", { name: "Add a remote archive" }).click();
  await expect(page).toHaveURL(/#\/settings\/remotes$/);
});

test("a remote list that fails says why", async ({ page, pacs }) => {
  pacs.on("GET", "/modalities", () => ({ status: 403, json: { Message: "Forbidden", Details: "the role may not use remote archives" } }));
  await page.goto("#/remote");
  await expect(page.getByRole("alert")).toContainText("Forbidden: the role may not use remote archives");
});

function jobs(pacs: Pacs, running = false) {
  const move = fixture("job-move.json");
  const sent = fixture("job-send.json");
  const live = running ? { ...move, State: "Running", Progress: 40, Remaining: 3, Completed: 2 } : move;
  pacs.on("GET", "/jobs", () => ({ json: [move.ID, sent.ID] }));
  pacs.on("GET", `/jobs/${move.ID}`, () => ({ json: live }));
  pacs.on("GET", `/jobs/${sent.ID}`, () => ({ json: sent }));
  return { move, sent };
}

test("the jobs page lists pulls and sends, newest first", async ({ page, pacs }) => {
  jobs(pacs);
  await page.goto("#/jobs");
  const rows = page.getByRole("table", { name: "Jobs" }).getByRole("row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(1)).toContainText("Send");
  await expect(rows.nth(1)).toContainText("C-STORE");
  await expect(rows.nth(2)).toContainText("Pull");
  await expect(rows.nth(2)).toContainText("hospital");
  await expect(rows.nth(2)).toContainText("Done");
  await expect(rows.nth(2)).toContainText("3 images");
});

test("a running job is shown with its progress and can be stopped", async ({ page, pacs }) => {
  const { move } = jobs(pacs, true);
  let cancelled = false;
  pacs.on("POST", `/jobs/${move.ID}/cancel`, () => {
    cancelled = true;
    return { status: 202, json: { ...move, State: "Running" } };
  });
  await page.goto("#/jobs");
  await expect(page.getByRole("progressbar", { name: `Job ${move.ID}` })).toHaveAttribute("aria-valuenow", "40");
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByText(`Job ${move.ID} asked to stop`)).toBeVisible();
  expect(cancelled).toBe(true);
});

test("a job that cannot be stopped says why", async ({ page, pacs }) => {
  const { move } = jobs(pacs, true);
  pacs.on("POST", `/jobs/${move.ID}/cancel`, () => ({ status: 409, json: { Message: "Conflict", Details: "the job is over already" } }));
  await page.goto("#/jobs");
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByText(`Job ${move.ID} could not be stopped`)).toBeVisible();
  await expect(page.getByText("Conflict: the job is over already")).toBeVisible();
});

test("a job the server dropped meanwhile is left out of the list", async ({ page, pacs }) => {
  const { move, sent } = jobs(pacs);
  pacs.on("GET", `/jobs/${move.ID}`, () => ({ status: 404, json: { Message: "Not found", Details: "no such job" } }));
  await page.goto("#/jobs");
  const rows = page.getByRole("table", { name: "Jobs" }).getByRole("row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1)).toContainText("Send");
  expect(sent.ID).toBeTruthy();
});

test("a jobs list that fails says why", async ({ page, pacs }) => {
  pacs.on("GET", "/jobs", () => ({ status: 500, json: { Message: "the index cannot be read" } }));
  await page.goto("#/jobs");
  await expect(page.getByRole("alert")).toContainText("the index cannot be read");
});

test("a study is sent to a remote archive from the study list", async ({ page, pacs }) => {
  remotes(pacs);
  let body: any = null;
  pacs.on("POST", "/modalities/hospital/store", (route) => {
    body = route.request().postDataJSON();
    return { status: 202, json: { ID: "3", Path: "/jobs/3" } };
  });
  pacs.on("GET", "/ui/api/studies", (_, url) => ({ json: fixture(url.searchParams.get("summary") ? "studies-summary.json" : "studies.json") }));
  await page.goto("#/studies");
  await page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Send to" }).hover();
  await page.getByRole("menuitem", { name: /hospital/ }).click();
  await expect(page.getByText("Sending to hospital")).toBeVisible();
  expect(body).toEqual({ StudyInstanceUID: [LITS], Asynchronous: true });
});

test("a send the server refuses says why", async ({ page, pacs }) => {
  remotes(pacs);
  pacs.on("POST", "/modalities/lab/store", () => ({ status: 409, json: fixture("remote-send-refused.json") }));
  await page.goto("#/studies");
  await page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Send to" }).hover();
  await page.getByRole("menuitem", { name: /lab/ }).click();
  await expect(page.getByText("Not sent to lab")).toBeVisible();
  await expect(page.getByText("lab is not an AE instances are sent to: list lab in PACS_DIMSE_MOVE_DESTINATIONS to send it studies")).toBeVisible();
});

test("an administrator pulls into a project, de-identified", async ({ page, pacs }) => {
  pacs.signedIn({ login: "admin", role: "admin", permissions: ["read", "write", "remote", "delete", "users"] });
  remotes(pacs);
  pacs.on("GET", "/projects", () => ({ json: ["liver"] }));
  let body: any = null;
  pacs.on("POST", "/modalities/hospital/move", (route) => {
    body = route.request().postDataJSON();
    return { status: 202, json: fixture("remote-move-started.json") };
  });
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" }).click();
  await page.getByLabel("Into").selectOption("liver");
  await page.getByRole("button", { name: "Pull 1 study" }).click();
  await expect(page.getByText("Into project liver, de-identified.")).toBeVisible();
  expect(body).toEqual({ PatientID: "LITS-0", StudyInstanceUID: [LITS], Asynchronous: true, Project: "liver" });
});

test("studies of several patients are pulled one patient at a time", async ({ page, pacs }) => {
  remotes(pacs);
  const bodies: any[] = [];
  pacs.on("POST", "/modalities/hospital/move", (route) => {
    bodies.push(route.request().postDataJSON());
    return { status: 202, json: fixture("remote-move-started.json") };
  });
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByRole("checkbox", { name: "Tick every study shown" }).click();
  await page.getByRole("button", { name: "Pull 3 studies" }).click();
  await expect(page.getByText("Pulling 3 studies from hospital")).toBeVisible();
  expect(bodies.map((b) => b.PatientID).sort()).toEqual(["1CT1", "4MR1", "LITS-0"]);
  expect(bodies.every((b) => b.StudyInstanceUID.length === 1)).toBe(true);
});

test("an account limited to projects pulls into its own, never into the archive", async ({ page, pacs }) => {
  pacs.signedIn({ login: "liver-reader", role: "radiologist", permissions: ["read", "remote"], projects: ["liver", "lung"] });
  remotes(pacs);
  let body: any = null;
  pacs.on("POST", "/modalities/hospital/move", (route) => {
    body = route.request().postDataJSON();
    return { status: 202, json: fixture("remote-move-started.json") };
  });
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByRole("row").filter({ hasText: "Three slices of LiTS volume 0" }).click();
  // its projects only: the archive itself is not offered, and GET /projects is never asked
  await expect(page.getByLabel("Into").getByRole("option")).toHaveText(["project liver (de-identified)", "project lung (de-identified)"]);
  await page.getByLabel("Into").selectOption("lung");
  await page.getByRole("button", { name: "Pull 1 study" }).click();
  await expect(page.getByText("Into project lung, de-identified.")).toBeVisible();
  expect(body.Project).toBe("lung");
  expect(pacs.asked.filter((a) => a.startsWith("GET /projects"))).toEqual([]);
});

test("a search stays with its archive: another one chosen is not searched", async ({ page, pacs }) => {
  remotes(pacs);
  await page.goto("#/remote?open=hospital");
  await page.getByLabel("Patient name").fill("smith");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("table", { name: "Studies of hospital" })).toBeVisible();
  await page.getByRole("list", { name: "Remote archives" }).getByRole("button", { name: /lab/ }).click();
  await expect(page.getByText("Search lab's studies")).toBeVisible();
  await page.waitForTimeout(500);
  expect(pacs.asked.filter((a) => a.startsWith("GET /modalities/lab/studies"))).toEqual([]);
});

test("Test every archive again does not search again", async ({ page, pacs }) => {
  remotes(pacs);
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("table", { name: "Studies of hospital" })).toBeVisible();
  const echoes = () => pacs.asked.filter((a) => a === "POST /modalities/hospital/echo").length;
  const before = echoes();
  await page.getByRole("button", { name: "Test every archive again" }).click();
  await expect.poll(echoes).toBeGreaterThan(before);
  expect(pacs.asked.filter((a) => a.startsWith("GET /modalities/hospital/studies"))).toHaveLength(1);
});

test("each warning of the archive is shown on its own", async ({ page, pacs }) => {
  remotes(pacs);
  pacs.on("GET", "/modalities/hospital/studies", () => ({
    json: fixture("remote-studies.json"),
    headers: { Warning: '299 pacs-rs "more studies match than the limit of 200", 299 pacs-rs "the archive did not match on every key"' },
  }));
  await page.goto("#/remote?open=hospital");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("status").filter({ hasText: "limit" })).toHaveText("more studies match than the limit of 200");
  await expect(page.getByRole("status").filter({ hasText: "every key" })).toHaveText("the archive did not match on every key");
});

test("an archive that does not exist is said so", async ({ page, pacs }) => {
  remotes(pacs);
  await page.goto("#/remote?open=typo");
  await expect(page.getByRole("status")).toContainText("There is no remote archive typo: hospital is shown.");
});

test("a role without remote archives has no Remote page in the rail", async ({ page, pacs }) => {
  pacs.signedIn({ login: "reader", role: "reader", permissions: ["read"] });
  await page.goto("#/studies");
  const rail = page.getByRole("navigation", { name: "pacs-rs" });
  await expect(rail.getByRole("link", { name: "Studies", exact: true })).toBeVisible();
  await expect(rail.getByRole("link", { name: "Remote", exact: true })).toHaveCount(0);
});
