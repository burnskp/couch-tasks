#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
state_dir="${CT_SYNC_TEST_DIR:-$repo_root/.sync-test}"
client_name="${CT_SYNC_TEST_CLIENT:-client-a}"

export CT_DB="${CT_DB:-$state_dir/$client_name.db}"
export CT_SYNC_URL="${CT_SYNC_URL:-https://localhost:8443}"
export CT_SYNC_CERT="${CT_SYNC_CERT:-$state_dir/client.crt}"
export CT_SYNC_KEY="${CT_SYNC_KEY:-$state_dir/client.key}"
export CT_SYNC_CA="${CT_SYNC_CA:-$state_dir/ca.crt}"

missing=0
for path in "$CT_SYNC_CERT" "$CT_SYNC_KEY" "$CT_SYNC_CA"; do
  if [[ ! -f "$path" ]]; then
    echo "missing sync test certificate file: $path" >&2
    missing=1
  fi
done

if [[ "$missing" -ne 0 ]]; then
  cat >&2 <<EOF

Run scripts/sync-test-server.sh first, or set these variables manually:
  CT_SYNC_TEST_DIR
  CT_SYNC_URL
  CT_SYNC_CERT
  CT_SYNC_KEY
  CT_SYNC_CA
EOF
  exit 1
fi

if [[ "$#" -eq 0 ]]; then
  set -- sync
fi

cat <<EOF
Running ct with sync test configuration:
  CT_DB:        $CT_DB
  CT_SYNC_URL:  $CT_SYNC_URL
  CT_SYNC_CERT: $CT_SYNC_CERT
  CT_SYNC_KEY:  $CT_SYNC_KEY
  CT_SYNC_CA:   $CT_SYNC_CA
  command:      ct $*
EOF

exec cargo run --bin ct -- "$@"
