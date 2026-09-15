{
  description = "STAR/FOLD — a stack-based terminal file manager";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      # Home-manager module, so STAR/FOLD can be installed and configured
      # declaratively the way the rest of a NixOS setup is.
      hmModule = { config, lib, pkgs, ... }:
        let cfg = config.programs.starfold;
        in {
          options.programs.starfold = {
            enable = lib.mkEnableOption "STAR/FOLD terminal file manager";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "The starfold package to use.";
            };
            theme = lib.mkOption {
              type = lib.types.str;
              default = "catppuccin-mocha";
              description = ''
                A built-in theme id, or the id of a file in
                ~/.local/starfold/themes. Ignored when stylix.enable is set.
              '';
            };
            stylix.enable = lib.mkEnableOption ''
              deriving a STAR/FOLD theme from the active Stylix base16 scheme,
              so the window matches the rest of the desktop automatically
            '';
            settings = lib.mkOption {
              type = lib.types.attrs;
              default = { };
              example = { ui.show_hidden = true; ops.trash = "never"; };
              description = ''
                Extra config.toml settings, merged last and table by table, so
                setting one key in [ui] leaves the rest of [ui] alone.
              '';
            };
          };

          config = lib.mkIf cfg.enable (lib.mkMerge [
            {
              home.packages = [ cfg.package ];
              # STAR/FOLD keeps everything under one directory rather than
              # spreading it across the XDG roots, so this is not
              # xdg.configFile.
              home.file.".local/starfold/config.toml".source =
                (pkgs.formats.toml { }).generate "starfold-config.toml" (
                  lib.recursiveUpdate
                    { ui.theme = if cfg.stylix.enable then "stylix" else cfg.theme; }
                    cfg.settings
                );
            }
            (lib.mkIf cfg.stylix.enable {
              home.file.".local/starfold/themes/stylix.toml".text =
                let c = config.lib.stylix.colors;
                in ''
                  # Generated from the active Stylix scheme.
                  [meta]
                  name = "Stylix"
                  id = "stylix"
                  variant = "${config.stylix.polarity}"

                  [base16]
                '' + lib.concatMapStringsSep "\n"
                  (n: ''base${n} = "#${c."base${n}"}"'')
                  [ "00" "01" "02" "03" "04" "05" "06" "07"
                    "08" "09" "0A" "0B" "0C" "0D" "0E" "0F" ]
                  + "\n";
            })
          ]);
        };
    in
    {
      homeManagerModules.starfold = hmModule;
      homeManagerModules.default = hmModule;
      overlays.default = final: prev: {
        starfold = self.packages.${final.stdenv.hostPlatform.system}.default;
      };
    }
    # Explicit rather than eachDefaultSystem, which would also claim systems
    # nobody has built this on. aarch64-darwin only, as in STAR/AMP: nixpkgs
    # 26.11 dropped x86_64-darwin outright, and naming it fails *evaluation*
    # with a release note rather than merely failing to build.
    // flake-utils.lib.eachSystem [
      "x86_64-linux"
      "aarch64-linux"
      "aarch64-darwin"
    ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # One version, read rather than repeated. scripts/check-version.sh
        # asserts the copies that cannot be derived (Cargo.lock, PKGBUILD).
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

        # There are no buildInputs and no nativeBuildInputs, which is worth
        # saying out loud because a file manager invites the guess that it
        # needs libmagic or a trash daemon's headers: file types are decided
        # from the name and the first bytes in Rust, and the trash is reached
        # by the freedesktop specification on Linux and through the Foundation
        # framework's Rust bindings on macOS. Nothing in the tree runs bindgen.
        # If that changes -- a dependency switching to a `-sys` crate, most
        # likely -- this is where pkg-config and the library go, and CI needs
        # the matching apt line.
        mkStarfold = { pkgsFor ? pkgs }:
          pkgsFor.rustPlatform.buildRustPackage {
            pname = "starfold";
            version = cargoToml.package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            # STAR/KIT comes from a git tag rather than from crates.io, and
            # `cargoLock.lockFile` alone cannot fetch it: nix wants a hash for
            # every source it downloads. Two ways to give it one.
            #
            # `outputHashes` is the reproducible one, and it means a new hash
            # to compute and commit on every STAR/KIT tag -- a second place the
            # version lives, which is exactly the kind of copy that goes stale
            # between the bump and the person who notices.
            #
            # This asks nix's builtin `fetchGit` for it instead. The tag is
            # immutable and `Cargo.lock` records the revision it resolved to,
            # so what is fetched is still pinned; what is given up is the
            # fixed-output hash, which means this fetch happens outside the
            # sandbox and a build with no network cannot do it. That is the
            # right trade here: the lockfile is the pin, and a stale hash
            # nobody bumped is a worse failure than a build that needs the
            # network it was already going to use.
            cargoLock.allowBuiltinFetchGit = true;

            # freedesktop assets, which mean nothing on macOS.
            postInstall = pkgsFor.lib.optionalString pkgsFor.stdenv.hostPlatform.isLinux ''
              install -Dm644 packaging/starfold.desktop \
                $out/share/applications/starfold.desktop
              install -Dm644 packaging/starfold.png \
                $out/share/icons/hicolor/256x256/apps/starfold.png
              install -Dm644 packaging/starfold.svg \
                $out/share/icons/hicolor/scalable/apps/starfold.svg
            '';

            meta = with pkgsFor.lib; {
              description = "A stack-based terminal file manager in the STAR family";
              homepage = "https://github.com/bstar/starfold";
              license = licenses.mit;
              mainProgram = "starfold";
              platforms = platforms.linux ++ platforms.darwin;
            };
          };
      in
      {
        packages.default = mkStarfold { };
        packages.starfold = mkStarfold { };

        # buildRustPackage runs `cargo test` as part of building the package,
        # so naming it here makes `nix flake check` cover the test suite too.
        checks = {
          inherit (self.packages.${system}) default;

          fmt = pkgs.runCommand "cargo-fmt"
            { nativeBuildInputs = [ pkgs.rustfmt ]; }
            ''
              cd ${./.}
              find src tests -name '*.rs' -print0 \
                | xargs -0 rustfmt --check --edition 2021
              touch $out
            '';
        };

        formatter = pkgs.nixpkgs-fmt;

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            # STAR/KIT is a git dependency, and cargo fetches it with the
            # git on PATH (`CARGO_NET_GIT_FETCH_WITH_CLI=true`, which is what
            # lets an `insteadOf` rewrite to ssh work). On a Mac with Xcode
            # selected, `/usr/bin/git` is a shim that asks xcrun where git
            # is, and inside this shell xcrun is nix's, which answers "tool
            # 'git' not found". Nix's own git, first on PATH, is the fix.
            git
            # scripts/check-version.sh reads `cargo metadata`.
            jq
            # The licence and advisory gate, so it is run before CI runs it.
            # `deny.toml` has one allowed git source, and the check that the
            # list still has exactly what it should is this command.
            cargo-deny
          ])
          # Only ever used to build a .deb, which only happens on Linux.
          ++ pkgs.lib.optional pkgs.stdenv.hostPlatform.isLinux pkgs.cargo-deb;

          shellHook = ''
            echo "STAR/FOLD devshell · rustc $(rustc --version | cut -d' ' -f2)"
          '';
        };
      });
}
