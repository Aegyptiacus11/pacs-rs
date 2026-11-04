#!/usr/bin/env bash
# What uploads of files with elements in UN and with lengths that lie cost a container limited
# to 1 GiB: the answers, the peak of its cgroup, whether it was OOM-killed. Linux with cgroup v2.
# Exits non-zero if the container was killed or restarted. See docs/benchmarks.md.
#
# un-vr-memory.sh <image> <dir from: cargo run -p pacs-core --features synthetic --example write_un_vr>
set -euo pipefail
image=$1 files=$2
name=un-vr-memory-$$
port=18048
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$name" --memory=1g --memory-swap=2g -p 127.0.0.1:$port:8080 "$image" >/dev/null
for i in $(seq 40); do
    curl -sf "http://127.0.0.1:$port/healthz" >/dev/null && break
    [ "$i" -lt 40 ] || { echo "the server did not start" >&2; exit 1; }
    sleep 0.25
done
id=$(docker inspect -f '{{.Id}}' "$name")
for pattern in 'un_sequence_*' 'un_uid_*' 'bad_length_*'; do
    # xargs ends non-zero when curl cannot connect, as it will if the server was killed.
    answers=$(find "$files" -name "$pattern.dcm" -print0 |
        xargs -0 -P6 -I{} curl -s -o /dev/null -w '%{http_code}\n' -m 120 --data-binary @{} \
            "http://127.0.0.1:$port/instances" | sort | uniq -c | tr -s ' \n' ' ') || true
    echo "$pattern:$answers"
done
# Gone with the container, if it was killed.
peak_file=/sys/fs/cgroup/system.slice/docker-$id.scope/memory.peak
if [ -r "$peak_file" ]; then
    echo "peak_MB $(($(cat "$peak_file") / 1048576))"
else
    echo "peak_MB unknown"
fi
state=$(docker inspect -f 'oom_killed={{.State.OOMKilled}} running={{.State.Running}} restarts={{.RestartCount}}' "$name")
echo "$state"
case $state in
    "oom_killed=false running=true"*) ;;
    *) echo "the container did not survive" >&2; exit 1 ;;
esac
