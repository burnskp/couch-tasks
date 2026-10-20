{
  description = "Couch Tasks CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;

      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = lib.genAttrs supportedSystems;

      pkgsFor = system: import nixpkgs {
        inherit system;
        config = {
          allowUnfree = true;
        };
      };
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          scriptPath = pkgs.lib.makeBinPath [
            pkgs.bash
            pkgs.coreutils
            pkgs.gnugrep
            pkgs.gnused
            pkgs.openssl
          ];
          commonRustPackageAttrs = {
            version = "0.1.0";

            src = pkgs.lib.cleanSource ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.openssl ];
          };
          installSyncCertificateScripts = ''
            install -Dm755 scripts/ct-sync-bootstrap-certs.sh $out/bin/ct-sync-bootstrap-certs
            install -Dm755 scripts/ct-sync-client-cert.sh $out/bin/ct-sync-client-cert

            wrapProgram $out/bin/ct-sync-bootstrap-certs \
              --prefix PATH : ${scriptPath}
            wrapProgram $out/bin/ct-sync-client-cert \
              --prefix PATH : ${scriptPath}
          '';
        in
        {
          default = self.packages.${system}.couch-tasks;

          couch-tasks = pkgs.rustPlatform.buildRustPackage (commonRustPackageAttrs // {
            pname = "couch-tasks";

            cargoBuildFlags = [ "--bins" ];
            cargoInstallFlags = [ "--bins" ];
            doCheck = false;

            postInstall = installSyncCertificateScripts + ''
              install -Dm644 completions/_ct $out/share/zsh/site-functions/_ct
            '';

            meta = with pkgs.lib; {
              description = "Local-first project-based task CLI";
              mainProgram = "ct";
              platforms = platforms.unix;
              license = licenses.mit;
              homepage = "https://github.com/burnskp/couch-tasks";
            };
          });

          couch-tasks-sync-server = pkgs.rustPlatform.buildRustPackage (commonRustPackageAttrs // {
            pname = "couch-tasks-sync-server";

            cargoBuildFlags = [ "--bin" "ct-sync-server" ];
            cargoInstallFlags = [ "--bin" "ct-sync-server" ];
            doCheck = false;

            postInstall = installSyncCertificateScripts;

            meta = with pkgs.lib; {
              description = "Couch Tasks sync server";
              mainProgram = "ct-sync-server";
              platforms = platforms.unix;
              license = licenses.mit;
              homepage = "https://github.com/burnskp/couch-tasks";
            };
          });
        });

      apps = forAllSystems (system: {
        default = self.apps.${system}.ct;
        ct = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/ct";
        };
      });

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              cargo-audit
              clippy
              git
              gitleaks
              just
              openssl
              prek
              rustc
              rustfmt
              zsh
            ];
          };
        });

      overlays.default = final: prev: {
        couch-tasks = self.packages.${final.system}.default;
      };

      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.couch-tasks-sync-server;
          package = self.packages.${pkgs.system}.couch-tasks-sync-server;
          user = "ct-sync";
          group = "ct-sync";
          stateDir = "/var/lib/ct-sync";
          dbDir = "${stateDir}/db";
          databasePath = "${dbDir}/tasks.db";
          certDir = "${stateDir}/certs";
          certFile = "${certDir}/server.crt";
          keyFile = "${certDir}/server.key";
          clientCaFile = "${certDir}/ca.crt";
          serverNameArgs = lib.concatMap (name: [ "--server-name" name ]) cfg.serverNames;
          bootstrapArgs = serverNameArgs;
        in
        {
          options.services.couch-tasks-sync-server = {
            enable = lib.mkEnableOption "Couch Tasks sync server";

            bind = lib.mkOption {
              type = lib.types.str;
              default = "0.0.0.0:51401";
              description = "Address and port for the sync server to bind.";
            };

            maxRequestBytes = lib.mkOption {
              type = lib.types.ints.positive;
              default = 16777216;
              description = "Maximum serialized sync request size in bytes. Larger limits increase potential memory use.";
            };

            serverNames = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ "localhost" "127.0.0.1" ];
              description = "DNS names or IP addresses to include as subjectAltName entries in the generated server certificate.";
            };
          };

          config = lib.mkIf cfg.enable {
            users.users.${user} = {
              isSystemUser = true;
              group = group;
              home = stateDir;
            };
            users.groups.${group} = { };

            systemd.tmpfiles.rules = [
              "d ${stateDir} 0750 root ${group} - -"
              "d ${dbDir} 0750 ${user} ${group} - -"
              "d ${certDir} 0750 root ${group} - -"
            ];

            system.activationScripts.couchTasksSyncCertSetup = {
              deps = [ "users" ];
              text = ''
                ${pkgs.coreutils}/bin/install -d -m 0750 -o root -g ${lib.escapeShellArg group} ${lib.escapeShellArg stateDir}
                ${pkgs.coreutils}/bin/install -d -m 0750 -o ${lib.escapeShellArg user} -g ${lib.escapeShellArg group} ${lib.escapeShellArg dbDir}
                ${pkgs.coreutils}/bin/install -d -m 0750 -o root -g ${lib.escapeShellArg group} ${lib.escapeShellArg certDir}
                ${package}/bin/ct-sync-bootstrap-certs ${lib.escapeShellArgs bootstrapArgs}
                ${pkgs.coreutils}/bin/printf '%s\n' \
                  '#!${pkgs.runtimeShell}' \
                  'exec ${package}/bin/ct-sync-client-cert "$@"' \
                  > ${lib.escapeShellArg "${certDir}/client-cert.sh"}
                ${pkgs.coreutils}/bin/chmod 0755 ${lib.escapeShellArg "${certDir}/client-cert.sh"}
                ${pkgs.coreutils}/bin/chown root:${lib.escapeShellArg group} ${lib.escapeShellArg "${certDir}/client-cert.sh"}
              '';
            };

            systemd.services.couch-tasks-sync-server = {
              description = "Couch Tasks sync server";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              serviceConfig = {
                Type = "simple";
                User = user;
                Group = group;
                ExecStart = "${package}/bin/ct-sync-server ${lib.escapeShellArgs [ "--bind" cfg.bind "--db" databasePath "--cert" certFile "--key" keyFile "--client-ca" clientCaFile "--max-request-bytes" (toString cfg.maxRequestBytes) ]}";
                Restart = "on-failure";
                RestartSec = "5s";
                NoNewPrivileges = true;
                PrivateTmp = true;
                ProtectHome = true;
                ProtectSystem = "strict";
                ReadWritePaths = [ dbDir ];
              };
            };
          };
        };

      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.couch-tasks;
        in
        {
          options.programs.couch-tasks = {
            enable = lib.mkEnableOption "Couch Tasks CLI";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              defaultText = lib.literalExpression "inputs.couch-tasks.packages.\${pkgs.system}.default";
              description = "The Couch Tasks package to install.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
          };
        };
    };
}
