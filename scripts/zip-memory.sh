#!/usr/bin/env bash
# Peak memory (VmHWM) of a pacs-server binary while it serves one study's ZIP; Linux only.
# See docs/benchmarks.md.
#
# zip-memory.sh <pacs-server binary> <label> <dir of .dcm of one study> <StudyInstanceUID>
set -euo pipefail
bin=$1 label=$2 files=$3 study=$4
d=$(mktemp -d)
PACS_LISTEN=127.0.0.1:18047 PACS_INDEX=$d/index.sqlite PACS_STORAGE=file:$d/files RUST_LOG=warn $bin & pid=$!
for i in $(seq 40); do curl -sf http://127.0.0.1:18047/healthz >/dev/null && break; sleep 0.25; done
ls "$files"/*.dcm | xargs -P4 -I{} curl -sf -o /dev/null --data-binary @{} http://127.0.0.1:18047/instances
before=$(grep VmHWM /proc/$pid/status | awk '{print $2}')
id=$(curl -sf -X POST -d '{"Level":"Study","Query":{"StudyInstanceUID":"'"$study"'"}}' http://127.0.0.1:18047/tools/find | python3 -c "import json,sys;print(json.load(sys.stdin)[0])")
t=$(curl -sf -o $d/a.zip -w "%{time_total} %{time_starttransfer}" http://127.0.0.1:18047/studies/$id/archive)
after=$(grep VmHWM /proc/$pid/status | awk '{print $2}')
python3 -c "import zipfile,sys;z=zipfile.ZipFile(sys.argv[1]);print('entries', len(z.namelist()), 'ok', z.testzip() is None)" $d/a.zip
echo "$label peak_rss_before_MB $((before/1024)) peak_rss_after_zip_MB $((after/1024)) total_s ${t% *} first_byte_s ${t#* }"
kill $pid; wait $pid 2>/dev/null || true; rm -rf $d
