#!/usr/bin/env bash
set -euo pipefail

cert_dir="${CT_SYNC_CERT_DIR:-/var/lib/ct-sync/certs}"
out_dir=""
days="${CT_SYNC_CLIENT_CERT_DAYS:-1825}"
client_cn=""
force=0
p12=0
p12_out=""
p12_password_file=""

usage() {
  cat <<'EOF'
Usage: ct-sync-client-cert [options] [CLIENT_CN]

Generate a Couch Tasks sync client certificate signed by the local sync server CA.
This command must be run as root because the CA private key is root-only.

Options:
  --cert-dir DIR   Directory containing ca.crt and ca.key (default: /var/lib/ct-sync/certs)
  --out-dir DIR    Directory for generated client files (default: cert dir)
  --cn CN          Client certificate common name; prompts if omitted
  --days DAYS               Certificate validity in days (default: 1825)
  --p12                     Also generate a PKCS#12 client bundle for Android
  --p12-out PATH            Output path for PKCS#12 bundle (default: client-<cn>.p12)
  --p12-password-file PATH  Read PKCS#12 export password from file instead of prompting
  --force                   Overwrite existing output files
  -h, --help                Show this help

Generated files are named client-<sanitized-cn>.key/.csr/.crt. With --p12, the
bundle is named client-<sanitized-cn>.p12 unless --p12-out is supplied. The
certificate subject preserves the requested CN.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cert-dir)
      cert_dir="${2:?missing value for --cert-dir}"
      shift 2
      ;;
    --out-dir)
      out_dir="${2:?missing value for --out-dir}"
      shift 2
      ;;
    --cn)
      client_cn="${2:?missing value for --cn}"
      shift 2
      ;;
    --days)
      days="${2:?missing value for --days}"
      shift 2
      ;;
    --p12)
      p12=1
      shift
      ;;
    --p12-out)
      p12=1
      p12_out="${2:?missing value for --p12-out}"
      shift 2
      ;;
    --p12-password-file)
      p12=1
      p12_password_file="${2:?missing value for --p12-password-file}"
      shift 2
      ;;
    --force)
      force=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      if [[ -n "$client_cn" ]]; then
        echo "unexpected extra argument: $1" >&2
        usage >&2
        exit 2
      fi
      client_cn="$1"
      shift
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
    echo "ct-sync-client-cert must be run as root" >&2
    exit 1
  fi
}

sanitize_filename() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9._-]+/-/g; s/^-+//; s/-+$//'
}

require_root
require_command openssl
require_command chmod
require_command mkdir
require_command sed
require_command tr

if [[ -z "$client_cn" ]]; then
  read -r -p "Client certificate common name: " client_cn
fi

if [[ -z "$client_cn" ]]; then
  echo "client certificate common name cannot be empty" >&2
  exit 1
fi

case "$client_cn" in
  *$'\n'*|*/*)
    echo "client certificate common name cannot contain newlines or slashes" >&2
    exit 1
    ;;
esac

out_dir="${out_dir:-$cert_dir}"
mkdir -p "$out_dir"

ca_key="$cert_dir/ca.key"
ca_cert="$cert_dir/ca.crt"

if [[ ! -f "$ca_key" || ! -f "$ca_cert" ]]; then
  echo "missing CA key/certificate in $cert_dir; expected ca.key and ca.crt" >&2
  exit 1
fi

safe_name="$(sanitize_filename "$client_cn")"
if [[ -z "$safe_name" ]]; then
  echo "client certificate common name must contain at least one filename-safe character" >&2
  exit 1
fi

client_key="$out_dir/client-$safe_name.key"
client_csr="$out_dir/client-$safe_name.csr"
client_cert="$out_dir/client-$safe_name.crt"
client_p12="${p12_out:-$out_dir/client-$safe_name.p12}"

if [[ -n "$p12_password_file" && ! -f "$p12_password_file" ]]; then
  echo "PKCS#12 password file does not exist: $p12_password_file" >&2
  exit 1
fi

if [[ "$force" -ne 1 ]]; then
  output_paths=("$client_key" "$client_csr" "$client_cert")
  if [[ "$p12" -eq 1 ]]; then
    output_paths+=("$client_p12")
  fi
  for path in "${output_paths[@]}"; do
    if [[ -e "$path" ]]; then
      echo "refusing to overwrite existing file: $path" >&2
      exit 1
    fi
  done
fi

echo "Generating client certificate for CN '$client_cn'"
openssl req -newkey rsa:4096 -nodes \
  -keyout "$client_key" \
  -out "$client_csr" \
  -subj "/CN=$client_cn" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
  -addext "extendedKeyUsage=clientAuth"

openssl x509 -req \
  -in "$client_csr" \
  -CA "$ca_cert" \
  -CAkey "$ca_key" \
  -CAcreateserial \
  -out "$client_cert" \
  -days "$days" \
  -copy_extensions copy

chmod 0600 "$client_key"
chmod 0644 "$client_csr" "$client_cert"
[[ -e "$cert_dir/ca.srl" ]] && chmod 0600 "$cert_dir/ca.srl"

if [[ "$p12" -eq 1 ]]; then
  echo "Generating PKCS#12 client bundle: $client_p12"
  p12_passout_args=()
  if [[ -n "$p12_password_file" ]]; then
    p12_passout_args=(-passout "file:$p12_password_file")
  fi
  openssl pkcs12 -export \
    -in "$client_cert" \
    -inkey "$client_key" \
    -certfile "$ca_cert" \
    -name "couch-tasks-client" \
    -out "$client_p12" \
    "${p12_passout_args[@]}"
  chmod 0600 "$client_p12"
fi

cat <<EOF
Client certificate generated:
  Client certificate: $client_cert
  Client key:         $client_key
  CA certificate:     $ca_cert

Copy the client certificate, client key, and CA certificate to the client, then run:
  ct sync --url https://YOUR_SERVER:51401 --cert '$client_cert' --key '$client_key' --ca '$ca_cert'

Or set CT_SYNC_CERT, CT_SYNC_KEY, and CT_SYNC_CA to those paths.
EOF

if [[ "$p12" -eq 1 ]]; then
  cat <<EOF

Android client bundle generated:
  PKCS#12 bundle:     $client_p12
  Server CA cert:     $ca_cert

Import the PKCS#12 bundle into the Android app for client authentication. The
app also needs the server CA certificate to verify the sync server unless it uses
another trust configuration.
EOF
fi
