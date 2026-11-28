// Records the answers the browser tests replay (tests/fixtures/), from a running pacs-server
// that holds only the DICOM files committed in this repository:
//
//   docker run -d --rm -p 127.0.0.1:18090:8080 -e PACS_LISTEN=0.0.0.0:8080 ghcr.io/aegyptiacus11/pacs-rs:<version>
//   for f in docs/sample/0000{1,2,3}.dcm crates/pacs-core/tests/data/pydicom/{CT_small,MR_small}.dcm; do
//     curl -fsS --data-binary @$f http://127.0.0.1:18090/instances; done
//   PACS_URL=http://127.0.0.1:18090 node tests/record.mjs
//
// It first puts labels on two studies (below), then records. roles.json and me-admin.json
// need the login on: with PACS_TOKEN set to an administrator's access token (POST /auth/login)
// of a server started with PACS_AUTH=true, they are recorded from it too.
//
// Stops with exit 1 on the first request that fails.
import { writeFile } from "node:fs/promises";

const base = process.env.PACS_URL;
if (!base) {
  console.error("PACS_URL is not set");
  process.exit(1);
}

async function ask(path, init) {
  const response = await fetch(new URL(path, base), init);
  if (!response.ok) {
    console.error(`${path}: ${response.status} ${response.statusText}`);
    process.exit(1);
  }
  return response;
}

// labels, as a user would put them: LiTS volume 0 "teaching" and "follow-up", CT1 "teaching"
const studies = (await (await ask("ui/api/studies?limit=50")).json()).Studies;
const named = (who) => studies.find((s) => s.PatientMainDicomTags.PatientName === who)?.ID;
for (const [who, label] of [["VOLUME 0^LITS", "teaching"], ["VOLUME 0^LITS", "follow-up"], ["CompressedSamples^CT1", "teaching"]]) {
  if (!named(who)) {
    console.error(`no study of ${who}`);
    process.exit(1);
  }
  await ask(`studies/${named(who)}/labels/${label}`, { method: "PUT" });
}

/** fixture file -> path asked of the server */
const answers = {
  "config.json": "ui/config.json",
  "studies.json": "ui/api/studies?sort=date&order=desc&limit=50",
  "studies-summary.json": "ui/api/studies?limit=1&summary=true",
  "studies-facets.json": "ui/api/studies/facets",
  "studies-follow-up.json": "ui/api/studies?label=follow-up&sort=date&order=desc&limit=50",
  "labels.json": "tools/labels",
  "patients.json": "ui/api/patients?sort=last&order=desc&limit=50",
  "server.json": "ui/api/server",
};

// each study's series and statistics, as a patient's record shows them
for (const id of await (await ask("studies")).json()) {
  answers[`study-${id}-series.json`] = `studies/${id}/series`;
  answers[`study-${id}-statistics.json`] = `studies/${id}/statistics`;
}

const token = process.env.PACS_TOKEN;
if (token) Object.assign(answers, { "roles.json": "roles", "me-admin.json": "auth/me" });

for (const [file, path] of Object.entries(answers)) {
  const response = await ask(path, token ? { headers: { Authorization: `Bearer ${token}` } } : {});
  await writeFile(new URL(`fixtures/${file}`, import.meta.url), JSON.stringify(await response.json(), null, 2) + "\n");
  console.log(`${file} <- ${path}`);
}
