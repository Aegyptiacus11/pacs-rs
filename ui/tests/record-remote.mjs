// Records the answers of the remote-archive routes the browser tests replay (tests/fixtures/
// remote-*.json), from two pacs-servers that know each other over DIMSE: "local", whose
// answers are kept, and "remote" (AE HOSPITAL), holding only the DICOM files of this
// repository. "lab" is listed on local but does not exist, for the failures.
//
//   docker network create uic-net
//   docker run -d --rm --name remote --network uic-net -p 127.0.0.1:18091:8080 \
//     -e PACS_LISTEN=0.0.0.0:8080 -e PACS_DIMSE_LISTEN=0.0.0.0:11112 -e PACS_DIMSE_AE_TITLE=HOSPITAL \
//     -e PACS_DIMSE_REMOTES=research=RESEARCH@local:11112 -e PACS_DIMSE_MOVE_DESTINATIONS=research \
//     ghcr.io/aegyptiacus11/pacs-rs:<version>
//   docker run -d --rm --name local --network uic-net -p 127.0.0.1:18090:8080 \
//     -e PACS_LISTEN=0.0.0.0:8080 -e PACS_DIMSE_LISTEN=0.0.0.0:11112 -e PACS_DIMSE_AE_TITLE=RESEARCH \
//     -e PACS_DIMSE_REMOTES=hospital=HOSPITAL@remote:11112,lab=LAB@nowhere:11112 \
//     -e PACS_DIMSE_MOVE_DESTINATIONS=hospital ghcr.io/aegyptiacus11/pacs-rs:<version>
//   for f in docs/sample/0000{1,2,3}.dcm crates/pacs-core/tests/data/pydicom/{CT_small,MR_small}.dcm; do
//     curl -fsS --data-binary @$f http://127.0.0.1:18091/instances; done
//   LOCAL_URL=http://127.0.0.1:18090 node tests/record-remote.mjs
//
// Stops with exit 1 when an answer is not the one expected.
import { writeFile } from "node:fs/promises";

const base = process.env.LOCAL_URL;
if (!base) {
  console.error("LOCAL_URL is not set");
  process.exit(1);
}

async function ask(method, path, body, expected) {
  const response = await fetch(new URL(path, base), {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  if (response.status !== expected) {
    console.error(`${method} ${path}: ${response.status}, expected ${expected}: ${await response.text()}`);
    process.exit(1);
  }
  return response.json();
}

async function keep(file, value) {
  await writeFile(new URL(`fixtures/${file}`, import.meta.url), JSON.stringify(value, null, 2) + "\n");
  console.log(file);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

await keep("remote-modalities.json", await ask("GET", "modalities?expand", null, 200));
await keep("remote-echo.json", await ask("POST", "modalities/hospital/echo", null, 200));
await keep("remote-echo-failed.json", await ask("POST", "modalities/lab/echo", null, 502));
const studies = await ask("GET", "modalities/hospital/studies", null, 200);
await keep("remote-studies.json", studies);
await keep("remote-studies-failed.json", await ask("GET", "modalities/lab/studies", null, 502));

const lits = studies.find((s) => s["00100020"]?.Value?.[0] === "LITS-0");
if (!lits) {
  console.error("the remote archive has no study of LITS-0");
  process.exit(1);
}
const uid = lits["0020000D"].Value[0];
const started = await ask("POST", "modalities/hospital/move", { StudyInstanceUID: uid, Asynchronous: true }, 202);
await keep("remote-move-started.json", started);
let job;
for (let i = 0; i < 50; i++) {
  job = await ask("GET", `jobs/${started.ID}`, null, 200);
  if (job.State !== "Running") break;
  await sleep(200);
}
if (job.State !== "Success") {
  console.error(`the move ended ${job.State}: ${job.Error}`);
  process.exit(1);
}
await keep("job-move.json", job);

const sent = await ask("POST", "modalities/hospital/store", { StudyInstanceUID: uid, Asynchronous: true }, 202);
for (let i = 0; i < 50; i++) {
  job = await ask("GET", `jobs/${sent.ID}`, null, 200);
  if (job.State !== "Running") break;
  await sleep(200);
}
if (job.State !== "Success") {
  console.error(`the send ended ${job.State}: ${job.Error}`);
  process.exit(1);
}
await keep("job-send.json", job);
await keep("remote-send-refused.json", await ask("POST", "modalities/lab/store", { StudyInstanceUID: uid }, 409));
await keep("jobs.json", await ask("GET", "jobs", null, 200));
