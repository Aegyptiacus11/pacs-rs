#!/usr/bin/env bash
# Times what a viewer and its backend do with one CT series: upload, series metadata,
# every frame, the study ZIP. See docs/benchmarks.md.
#
# bench.sh <label> <rest root> <dicomweb root> <dir of .dcm>
# The .dcm files are those of
#   cargo run --release -p pacs-core --features synthetic --example write_synthetic -- ct 77 200 512
set -euo pipefail
label=$1 rest=$2 web=$3 dir=$4
study=2.25.77100000 series=2.25.77200000
accept_frames='multipart/related; type=application/octet-stream; transfer-syntax=*'
now() { date +%s.%N; }
since() { echo "$(now) - $1" | bc; }

t=$(now)
ls "$dir"/*.dcm | xargs -P4 -I{} curl -sf -o /dev/null --data-binary @{} "$rest/instances"
echo "$label ingest_200_P4_s $(since "$t")"

for i in 1 2 3 4 5 6; do
  curl -sf -o /dev/null -w "%{time_total}\n" -H 'Accept: application/dicom+json' \
    "$web/studies/$study/series/$series/metadata"
done | python3 -c "
import sys, statistics
t = [float(x) for x in sys.stdin]
print('$label metadata_first_s', t[0])
print('$label metadata_median_s', statistics.median(t[1:]))"

uids=$(curl -sf -H 'Accept: application/dicom+json' "$web/studies/$study/series/$series/instances" \
  | python3 -c "import json,sys;print(' '.join(i['00080018']['Value'][0] for i in json.load(sys.stdin)))")

t=$(now)
for u in $uids; do
  curl -sf -o /dev/null -H "Accept: $accept_frames" "$web/studies/$study/series/$series/instances/$u/frames/1"
done
echo "$label frames_200_sequential_s $(since "$t")"

t=$(now)
echo $uids | tr ' ' '\n' | xargs -P8 -I{} curl -sf -o /dev/null -H "Accept: $accept_frames" \
  "$web/studies/$study/series/$series/instances/{}/frames/1"
echo "$label frames_200_P8_s $(since "$t")"

id=$(curl -sf -X POST -d "{\"Level\":\"Study\",\"Query\":{\"StudyInstanceUID\":\"$study\"}}" "$rest/tools/find" \
  | python3 -c "import json,sys;print(json.load(sys.stdin)[0])")
curl -sf -o /dev/null -w "$label archive_s %{time_total} archive_bytes %{size_download}\n" "$rest/studies/$id/archive"
