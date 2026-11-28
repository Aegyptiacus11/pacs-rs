// Records the answers the browser tests replay (tests/fixtures/), from a running pacs-server
// that holds only the DICOM files committed in this repository:
//
//   docker run -d --rm -p 127.0.0.1:18090:8080 -e PACS_LISTEN=0.0.0.0:8080 ghcr.io/aegyptiacus11/pacs-rs:<version>
//   for f in docs/sample/0000{1,2,3}.dcm crates/pacs-core/tests/data/pydicom/{CT_small,MR_small}.dcm; do
//     curl -fsS --data-binary @$f http://127.0.0.1:18090/instances; done
//   PACS_URL=http://127.0.0.1:18090 node tests/record.mjs
//
// Stops with exit 1 on the first request that fails.
import { writeFile } from "node:fs/promises";

const base = process.env.PACS_URL;
if (!base) {
  console.error("PACS_URL is not set");
  process.exit(1);
}

/** fixture file -> path asked of the server */
const answers = {
  "config.json": "ui/config.json",
  "studies.json": "ui/api/studies?sort=date&order=desc&limit=50",
  "studies-summary.json": "ui/api/studies?limit=1&summary=true",
  "studies-facets.json": "ui/api/studies/facets",
  "patients.json": "ui/api/patients?sort=last&order=desc&limit=50",
  "server.json": "ui/api/server",
};

for (const [file, path] of Object.entries(answers)) {
  const response = await fetch(new URL(path, base));
  if (!response.ok) {
    console.error(`${path}: ${response.status} ${response.statusText}`);
    process.exit(1);
  }
  await writeFile(new URL(`fixtures/${file}`, import.meta.url), JSON.stringify(await response.json(), null, 2) + "\n");
  console.log(`${file} <- ${path}`);
}
