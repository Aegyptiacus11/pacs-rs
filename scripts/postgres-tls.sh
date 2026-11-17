#!/usr/bin/env bash
# Starts a Postgres that speaks TLS, for the tests of PACS_INDEX_TLS, and prints the two
# variables they read. Its certificate is made here, signed by an authority made here too,
# and names the address 127.0.0.1 only: `localhost` is the name it must be refused under.
#
#   scripts/postgres-tls.sh <directory for the certificates> [port] [image]
#
# Needs docker and openssl. The container is `pacs-pg-tls`; an older one is replaced.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
  echo "usage: postgres-tls.sh <directory> [port] [image]" >&2
  exit 2
fi
dir=$1
port=${2:-15433}
image=${3:-postgres:18.6-alpine}
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)

# an authority, and a certificate of the server it signs, for 127.0.0.1
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=pacs-rs test authority" \
  -keyout "$dir/ca.key" -out "$dir/ca.crt" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=pacs-rs test postgres" \
  -keyout "$dir/server.key" -out "$dir/server.csr" 2>/dev/null
printf 'subjectAltName=IP:127.0.0.1\n' > "$dir/san.cnf"
openssl x509 -req -in "$dir/server.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" \
  -CAcreateserial -days 2 -extfile "$dir/san.cnf" -out "$dir/server.crt" 2>/dev/null
# another authority, which signed nothing: the one a client must not accept the server by
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=another authority" \
  -keyout "$dir/other.key" -out "$dir/other.crt" 2>/dev/null
chmod 644 "$dir"/*.crt "$dir/server.key"

docker rm -f pacs-pg-tls >/dev/null 2>&1 || true
# Postgres wants its key owned by itself and closed to others: copied in before it starts
docker run -d --name pacs-pg-tls -p "127.0.0.1:$port:5432" -e POSTGRES_PASSWORD=test \
  -v "$dir:/certs:ro" --entrypoint sh "$image" -c '
    cp /certs/server.crt /certs/server.key /tmp/ &&
    chown postgres /tmp/server.crt /tmp/server.key && chmod 600 /tmp/server.key &&
    exec docker-entrypoint.sh postgres -c ssl=on \
      -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key' >/dev/null

for _ in $(seq 1 60); do
  if docker exec pacs-pg-tls pg_isready -U postgres -q 2>/dev/null; then
    # the first start runs initdb and restarts once: wait for the one that stays
    sleep 2
    if docker exec pacs-pg-tls pg_isready -U postgres -q 2>/dev/null; then
      echo "PACS_TEST_POSTGRES_TLS_URL=postgres://postgres:test@127.0.0.1:$port"
      echo "PACS_TEST_POSTGRES_TLS_CERTS=$dir"
      exit 0
    fi
  fi
  sleep 1
done
echo "postgres-tls.sh: the server did not come up" >&2
docker logs --tail 20 pacs-pg-tls >&2
exit 1
