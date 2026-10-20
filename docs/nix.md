# Nix setup

The flake supports `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, and
`aarch64-darwin`. The service module requires NixOS. Enable Nix flakes and the
`nix-command` feature before using these commands.

## Build and run

From the repository checkout:

```sh
nix build .#
./result/bin/ct --help
nix run .# -- quick buy milk
```

The default package includes `ct`, `ct-sync-server`, certificate helpers, and
zsh completion. To install in your user profile:

```sh
nix profile install github:burnskp/couch-tasks
```

## Home Manager

Add Couch Tasks to your flake inputs:

```nix
inputs.couch-tasks.url = "github:burnskp/couch-tasks";
```

Pass `inputs` to your Home Manager configuration through `extraSpecialArgs`,
then import the module:

```nix
{ inputs, ... }:
{
  imports = [ inputs.couch-tasks.homeManagerModules.default ];
  programs.couch-tasks.enable = true;
}
```

Alternatively, add the package directly:

```nix
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.couch-tasks.packages.${pkgs.system}.default ];
}
```

Apply your Home Manager configuration. The package installs zsh completion in
`share/zsh/site-functions/_ct`; enable Home Manager's `programs.zsh.enableCompletion`
to load it.

## NixOS sync server

Add the flake input above and pass `inputs` through your NixOS configuration's
`specialArgs`. Import the module and configure the server:

```nix
{ inputs, ... }:
{
  imports = [ inputs.couch-tasks.nixosModules.default ];
  services.couch-tasks-sync-server = {
    enable = true;
    bind = "0.0.0.0:51401";
    serverNames = [ "tasks.example.com" ];
  };
  networking.firewall.allowedTCPPorts = [ 51401 ];
}
```

Replace the hostname with your server address, then rebuild and switch your
NixOS configuration. Include every DNS name or IP clients use in `serverNames`.
The module does not open the firewall automatically.

| Option | Default |
| --- | --- |
| `bind` | `0.0.0.0:51401` |
| `serverNames` | `[ "localhost" "127.0.0.1" ]` |
| `maxRequestBytes` | `16777216` (16 MiB), must be positive |

The service runs as `ct-sync:ct-sync`. Its database is
`/var/lib/ct-sync/db/tasks.db`; certificates are in `/var/lib/ct-sync/certs`.
Activation generates missing CA and server certificates, valid for five years,
and keeps the CA private key root-only. Private keys must remain outside Nix
expressions and the Nix store.

Existing certificate/key pairs are reused. To change server names after setup,
replace or remove both `server.crt` and `server.key` in the certificate directory,
then rebuild/switch to generate the replacement and restart the service. Retain
`ca.crt` and `ca.key` to preserve trust for existing clients. Replace expired
certificates before they prevent connections.

Check service status and logs:

```sh
sudo systemctl status couch-tasks-sync-server.service
sudo journalctl -u couch-tasks-sync-server.service -f
```

For HTTP 413 errors, increase `services.couch-tasks-sync-server.maxRequestBytes`,
rebuild/switch, restart the service, and retry sync. See [sync troubleshooting](sync.md#troubleshooting)
for other failures.

## Provision clients

On the server, generate a signed certificate for each device:

```sh
sudo /var/lib/ct-sync/certs/client-cert.sh --cn laptop
```

The helper writes `client-laptop.crt` and `client-laptop.key` in the certificate
directory. Transfer these and `ca.crt` securely to the client, renaming them to
`client.crt`, `client.key`, and `server-ca.crt` respectively. Keep the private key
accessible only to its owner. Follow [connect a client](sync.md#connect-a-client)
for destination paths and URL configuration. Do not copy the CA private key.

For a device that needs a PKCS#12 bundle:

```sh
sudo /var/lib/ct-sync/certs/client-cert.sh \
  --cn phone --p12 --p12-out /tmp/client-phone.p12
```

The helper prompts for an export password. Transfer the bundle and public
`ca.crt` using the device's certificate provisioning workflow, then remove the
temporary bundle after provisioning. Generated client certificates are valid
for five years by default.
