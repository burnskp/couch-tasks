# Sync setup

Sync is optional. Each device keeps its own local tasks; run `ct sync` on each
device to send and receive changes. Sync includes notes, task order, completion,
project moves, and deletions.

The server uses mutual TLS: clients need a signed certificate and private key,
and a CA certificate to verify the server. All accepted clients share the same
tasks. There are no separate user accounts or private per-user projects.

## Connect a client

Obtain the server URL, a client certificate, its private key, and the server's
CA certificate from the server operator. Place the files at these default paths:

```text
~/.local/share/couch-tasks/client.crt
~/.local/share/couch-tasks/client.key
~/.local/share/couch-tasks/server-ca.crt
```

Keep the private key accessible only to its owner. If `XDG_DATA_HOME` is set,
the default directory is `$XDG_DATA_HOME/couch-tasks` instead.

Create `~/.config/couch-tasks/config.json` (and its parent directory if needed):

```json
{
  "sync": {
    "url": "https://tasks.example.com:51401"
  }
}
```

If `XDG_CONFIG_HOME` is set, use `$XDG_CONFIG_HOME/couch-tasks/config.json`.
Then run:

```sh
ct sync --verbose
```

Success prints `Synced to checkpoint N.`; the number changes as data is synced.
Without `--verbose` (or `-v`), success is silent. Run sync on your other device to
receive changes, and sync again after making changes there.

To use other certificate locations, add `cert`, `key`, and `ca` under `sync` in
the configuration file. Paths beginning with `~/` are supported. You can also
supply everything on the command line:

```sh
ct sync --verbose \
  --url https://tasks.example.com:51401 \
  --cert ~/.local/share/couch-tasks/client.crt \
  --key ~/.local/share/couch-tasks/client.key \
  --ca ~/.local/share/couch-tasks/server-ca.crt
```

| Setting | Flag | Environment variable |
| --- | --- | --- |
| Server URL | `--url` | `CT_SYNC_URL` |
| Client certificate | `--cert` | `CT_SYNC_CERT` |
| Client private key | `--key` | `CT_SYNC_KEY` |
| Server CA certificate | `--ca` | `CT_SYNC_CA` |

Settings are resolved from flags, then environment variables, then the config
file, then the default certificate paths. The URL has no default. Use the final
HTTPS address: HTTP and redirects are rejected.

## Run a server

For automatic certificate setup and a managed service, see [NixOS setup](nix.md#nixos-sync-server).
For a temporary local trial, see [try sync locally](#try-sync-locally).

For manual hosting, install `ct-sync-server` using the [Cargo installation](../README.md#install).
Prepare a writable database directory and PEM certificates before starting:

- A server certificate and private key, valid for every DNS name or IP clients use.
- A client CA certificate whose signed client certificates the server will trust.
- A client certificate and private key for each device, plus the CA certificate
  clients need to verify the server. One CA can sign both server and client certificates.

Run under an account that can read the server key and write the database directory:

```sh
ct-sync-server \
  --bind 0.0.0.0:51401 \
  --db /var/lib/ct-sync/db/tasks.db \
  --cert /var/lib/ct-sync/certs/server.crt \
  --key /var/lib/ct-sync/certs/server.key \
  --client-ca /var/lib/ct-sync/certs/ca.crt
```

Allow the chosen TCP port through your firewall if clients connect remotely.
The server runs in the foreground; use your process supervisor for persistent
hosting. Stop a foreground instance with Ctrl-C.

| Flag | Environment variable | Standalone default |
| --- | --- | --- |
| `--bind` | `CT_SYNC_BIND` | `127.0.0.1:8443` |
| `--db` | `CT_SYNC_DB` | `sync-server.db` in the working directory |
| `--cert` | `CT_SYNC_SERVER_CERT` | Required |
| `--key` | `CT_SYNC_SERVER_KEY` | Required |
| `--client-ca` | `CT_SYNC_CLIENT_CA` | Required |
| `--max-request-bytes` | `CT_SYNC_MAX_REQUEST_BYTES` | `16777216` (16 MiB) |

Flags override environment variables. Each sync uploads the full local task
snapshot, including completed tasks and deletion records, so the request limit
must accommodate that data. Set a positive byte count to change it.

Server logs go to stdout/stderr. To adjust verbosity, prefix the server command
with `RUST_LOG=couch_tasks::sync_server=debug,ct_sync_server=info`. Request logs
identify the client IP and certificate common name; they omit task text and key
contents.

To check connectivity and certificates, use an authenticated health request:

```sh
curl --cacert server-ca.crt --cert client.crt --key client.key \
  https://tasks.example.com:51401/healthz
```

It should return `ok`. A request without a trusted client certificate fails.

## Try sync locally

This walkthrough uses the checkout's helper scripts, Cargo, OpenSSL, and two
terminals. It creates temporary certificates and databases without touching your
normal tasks. Run both terminals from the repository root.

In terminal 1, create a scratch directory and start the server:

```sh
sync_demo_dir="$(mktemp -d)"
echo "$sync_demo_dir"
CT_SYNC_TEST_DIR="$sync_demo_dir" \
CT_SYNC_BIND=127.0.0.1:8443 \
CT_SYNC_DB="$sync_demo_dir/server.db" \
CT_SYNC_URL=https://localhost:8443 \
  bash scripts/sync-test-server.sh
```

Leave it running. The script prints the path to `client.env`. In terminal 2,
replace `/tmp/your-directory` below with the scratch directory printed in terminal 1:

```sh
sync_demo_dir=/tmp/your-directory
source "$sync_demo_dir/client.env"
CT_DB="$sync_demo_dir/client-a.db" bash scripts/sync-test-client.sh quick buy milk
CT_DB="$sync_demo_dir/client-a.db" bash scripts/sync-test-client.sh sync --verbose
CT_DB="$sync_demo_dir/client-b.db" bash scripts/sync-test-client.sh sync --verbose
CT_DB="$sync_demo_dir/client-b.db" bash scripts/sync-test-client.sh list
```

Client B should show `buy milk`. Confirm deletion reaches the other device:

```sh
CT_DB="$sync_demo_dir/client-a.db" bash scripts/sync-test-client.sh list
CT_DB="$sync_demo_dir/client-a.db" bash scripts/sync-test-client.sh delete 1
CT_DB="$sync_demo_dir/client-a.db" bash scripts/sync-test-client.sh sync
CT_DB="$sync_demo_dir/client-b.db" bash scripts/sync-test-client.sh sync
CT_DB="$sync_demo_dir/client-b.db" bash scripts/sync-test-client.sh list
```

The last list should report no pending tasks. The helper prints configuration
information before each command; that output is not JSON.

Stop the server with Ctrl-C in terminal 1, then remove its scratch directory:

```sh
rm -rf "$sync_demo_dir"
```

Close terminal 2 to discard the temporary environment settings. Test certificates
expire after one day; start with a new scratch directory for another trial.

## Troubleshooting

| Symptom | Action |
| --- | --- |
| Missing sync URL | Set `sync.url`, `CT_SYNC_URL`, or `--url`. |
| Missing certificate/key | Check the configured paths and read permissions. |
| Certificate verification failure | Check expiry, CA trust, and that the URL hostname/IP is in the server certificate. Check that the client certificate was signed by the server's client CA. |
| Connection refused or timeout | Check that the server is running, the bind address and port are correct, and the firewall allows access. |
| Local data changed during sync | Run `ct sync` again after local edits finish. Your local changes are retained. |
| HTTP 413 | Increase the server's `--max-request-bytes` or NixOS `maxRequestBytes`, restart the service, and retry. Larger limits use more memory; retries alone cannot fix the limit. |
| HTTP 400 or 409 | Read the reported validation/conflict error and inspect server logs. Repeated retries cannot repair invalid data or a checkpoint ahead of a restored server. Preserve your database before attempting recovery; there is no CLI repair command. |
| HTTP 500 | Inspect server logs and database access permissions before retrying. |

Concurrent edits to the same task can overwrite one another; sync before editing
on another device to reduce conflicts. Sync is not a backup or edit history.
