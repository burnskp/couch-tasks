#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
state_dir="${CT_SYNC_TEST_DIR:-$repo_root/.sync-test}"
bind_addr="${CT_SYNC_BIND:-127.0.0.1:8443}"
server_db="${CT_SYNC_DB:-$state_dir/server.db}"
server_san="${CT_SYNC_TEST_SERVER_SAN:-DNS:localhost,IP:127.0.0.1}"
url="${CT_SYNC_URL:-https://localhost:${bind_addr##*:}}"

ca_key="$state_dir/ca.key"
ca_cert="$state_dir/ca.crt"
server_key="$state_dir/server.key"
server_csr="$state_dir/server.csr"
server_cert="$state_dir/server.crt"
client_key="$state_dir/client.key"
client_csr="$state_dir/client.csr"
client_cert="$state_dir/client.crt"
client_env="$state_dir/client.env"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

generate_certs() {
  mkdir -p "$state_dir"

  if [[ -f "$ca_cert" && -f "$server_cert" && -f "$server_key" && -f "$client_cert" && -f "$client_key" ]]; then
    echo "Using existing test certificates in $state_dir"
    return
  fi

  echo "Generating test certificates in $state_dir"
  rm -f \
    "$ca_key" "$ca_cert" "$state_dir/ca.srl" \
    "$server_key" "$server_csr" "$server_cert" \
    "$client_key" "$client_csr" "$client_cert"

  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$ca_key" \
    -out "$ca_cert" \
    -subj "/CN=Couch Tasks Local Test CA" \
    -days 1

  openssl req -newkey rsa:2048 -nodes \
    -keyout "$server_key" \
    -out "$server_csr" \
    -subj "/CN=localhost" \
    -addext "subjectAltName=$server_san"

  openssl x509 -req \
    -in "$server_csr" \
    -CA "$ca_cert" \
    -CAkey "$ca_key" \
    -CAcreateserial \
    -out "$server_cert" \
    -days 1 \
    -copy_extensions copy

  openssl req -newkey rsa:2048 -nodes \
    -keyout "$client_key" \
    -out "$client_csr" \
    -subj "/CN=couch-tasks-local-client"

  openssl x509 -req \
    -in "$client_csr" \
    -CA "$ca_cert" \
    -CAkey "$ca_key" \
    -CAcreateserial \
    -out "$client_cert" \
    -days 1
}

write_client_env() {
  cat >"$client_env" <<EOF
export CT_SYNC_TEST_DIR="$state_dir"
export CT_SYNC_URL="$url"
export CT_SYNC_CERT="$client_cert"
export CT_SYNC_KEY="$client_key"
export CT_SYNC_CA="$ca_cert"
EOF
}

require_command openssl
generate_certs
write_client_env

cat <<EOF
Sync test server configuration:
  state dir:  $state_dir
  bind:       $bind_addr
  url:        $url
  database:   $server_db
  client env: $client_env

In another terminal, run:
  source "$client_env"
  scripts/sync-test-client.sh quick write local sync test
  scripts/sync-test-client.sh sync

Starting ct-sync-server...
EOF

exec cargo run --bin ct-sync-server -- \
  --bind "$bind_addr" \
  --db "$server_db" \
  --cert "$server_cert" \
  --key "$server_key" \
  --client-ca "$ca_cert"
