#!/usr/bin/env bash
set -euo pipefail

cert_dir="${CT_SYNC_CERT_DIR:-/var/lib/ct-sync/certs}"
user="${CT_SYNC_USER:-ct-sync}"
group="${CT_SYNC_GROUP:-ct-sync}"
days="${CT_SYNC_CERT_DAYS:-1825}"
server_names=()
server_sans=()

usage() {
  cat <<'EOF'
Usage: ct-sync-bootstrap-certs [options]

Create missing Couch Tasks sync server CA and server certificates.

Options:
  --cert-dir DIR        Certificate directory (default: /var/lib/ct-sync/certs)
  --user USER           Service user to validate before generation (default: ct-sync)
  --group GROUP         Service group granted read access to server material (default: ct-sync)
  --server-name NAME    Add DNS/IP subjectAltName for the server certificate
  --server-san SAN      Add raw subjectAltName entry, e.g. DNS:tasks.example.com
  --days DAYS           Certificate validity in days (default: 1825)
  -h, --help            Show this help

Generated CA certificates use CN "couch-tasks sync CA" and server certificates
use CN "couch-tasks sync server". Existing complete certificate sets are reused.
Partial key/certificate pairs fail rather than being overwritten. The CA private
key remains root-only; the service user is given access only to the public CA
certificate and server certificate material.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cert-dir)
      cert_dir="${2:?missing value for --cert-dir}"
      shift 2
      ;;
    --user)
      user="${2:?missing value for --user}"
      shift 2
      ;;
    --group)
      group="${2:?missing value for --group}"
      shift 2
      ;;
    --server-name)
      server_names+=("${2:?missing value for --server-name}")
      shift 2
      ;;
    --server-san)
      server_sans+=("${2:?missing value for --server-san}")
      shift 2
      ;;
    --days)
      days="${2:?missing value for --days}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

require_root() {
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "ct-sync-bootstrap-certs must be run as root" >&2
    exit 1
  fi
}

require_service_user() {
  if ! id -u "$user" >/dev/null 2>&1; then
    echo "service user does not exist: $user" >&2
    exit 1
  fi
}

is_ip_address() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ || "$1" == *:* ]]
}

san_for_name() {
  if is_ip_address "$1"; then
    printf 'IP:%s' "$1"
  else
    printf 'DNS:%s' "$1"
  fi
}

join_by_comma() {
  local IFS=,
  echo "$*"
}

ensure_pair_state() {
  local key="$1"
  local cert="$2"
  local label="$3"

  if [[ -e "$key" && ! -e "$cert" ]]; then
    echo "refusing to continue: found $label key without certificate: $key" >&2
    exit 1
  fi
  if [[ ! -e "$key" && -e "$cert" ]]; then
    echo "refusing to continue: found $label certificate without key: $cert" >&2
    exit 1
  fi
}

set_ca_permissions() {
  chown root:root "$ca_key"
  chmod 0600 "$ca_key"
  chown root:"$group" "$ca_cert"
  chmod 0644 "$ca_cert"
}

set_server_permissions() {
  chown root:"$group" "$server_key"
  chmod 0640 "$server_key"
  chown root:"$group" "$server_cert"
  chmod 0644 "$server_cert"
  [[ -e "$serial_file" ]] && chown root:root "$serial_file"
  [[ -e "$serial_file" ]] && chmod 0600 "$serial_file"
}

set_directory_permissions() {
  chown root:"$group" "$cert_dir"
  chmod 0750 "$cert_dir"
}

require_root
require_command openssl
require_command chmod
require_command chown
require_command id
require_command mkdir
require_service_user

mkdir -p "$cert_dir"

ca_key="$cert_dir/ca.key"
ca_cert="$cert_dir/ca.crt"
server_key="$cert_dir/server.key"
server_csr="$cert_dir/server.csr"
server_cert="$cert_dir/server.crt"
serial_file="$cert_dir/ca.srl"

ensure_pair_state "$ca_key" "$ca_cert" "CA"
ensure_pair_state "$server_key" "$server_cert" "server"

if [[ ! -e "$ca_key" && ! -e "$ca_cert" ]]; then
  echo "Generating CA certificate in $cert_dir"
  openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout "$ca_key" \
    -out "$ca_cert" \
    -subj "/CN=couch-tasks sync CA" \
    -days "$days" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign"
  set_ca_permissions
else
  echo "Using existing CA certificate in $cert_dir"
  set_ca_permissions
fi

if [[ ! -e "$server_key" && ! -e "$server_cert" ]]; then
  if [[ ${#server_names[@]} -eq 0 && ${#server_sans[@]} -eq 0 ]]; then
    server_names=(localhost 127.0.0.1)
  fi

  san_entries=()
  for name in "${server_names[@]}"; do
    san_entries+=("$(san_for_name "$name")")
  done
  for san in "${server_sans[@]}"; do
    san_entries+=("$san")
  done
  subject_alt_name="$(join_by_comma "${san_entries[@]}")"

  echo "Generating server certificate in $cert_dir"
  openssl req -newkey rsa:4096 -nodes \
    -keyout "$server_key" \
    -out "$server_csr" \
    -subj "/CN=couch-tasks sync server" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" \
    -addext "subjectAltName=$subject_alt_name"

  openssl x509 -req \
    -in "$server_csr" \
    -CA "$ca_cert" \
    -CAkey "$ca_key" \
    -CAcreateserial \
    -out "$server_cert" \
    -days "$days" \
    -copy_extensions copy

  rm -f "$server_csr"
  set_server_permissions
else
  echo "Using existing server certificate in $cert_dir"
  set_server_permissions
fi

set_directory_permissions

cat <<EOF
Certificate bootstrap complete:
  CA certificate:     $ca_cert
  CA key:             $ca_key
  Server certificate: $server_cert
  Server key:         $server_key
EOF
