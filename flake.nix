{
  description = "balthasar, and its Rust development shell";

  nixConfig = {
    extra-substituters = [ "https://cache.bresilla.com/bresilla" ];
    extra-trusted-public-keys = [
      "bresilla:zsecaZYIrjCJQS4AuGPmEVU5PRkiMmB/Mds3KaCzTvE="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=4c1018dae018162ec878d42fec712642d214fdfa";
    flake-utils.url = "github:numtide/flake-utils";
    # For the musl target's standard library, and nothing else.
    #
    # nixpkgs ships `rustc` with std for the host only, so `--target x86_64-unknown-linux-musl`
    # fails on "can't find crate for `core`". Its answer is `pkgsCross.musl64.rustc`, which
    # compiles a whole cross rustc from source -- hours, for a file we already have. This
    # overlay hands over the official prebuilt std for a named target instead.
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    { nixpkgs, flake-utils, rust-overlay, ... }:
    let
      manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The static `balthasar` binary, with the shipped configuration beside it.
      packaged = flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          lib = pkgs.lib;
          balthasar = pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "balthasar";
            inherit (manifest.workspace.package) version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
                ./config
                ./vendor
              ];
            };

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "luna-0.5.0" = "sha256-YJsEKm7UykhqNQjhLPBgpmCHQZQhDLFVAENSD43SyAw=";
              };
            };
            # SQLite is compiled from rusqlite's bundled amalgamation by the static C compiler.
            cargoBuildFlags = [
              "-p"
              "balthasar"
              "--bin"
              "balthasar"
            ];
            stripAllList = [ "bin" ];

            doCheck = false;
            # Source paths baked into the binary name /build, wherever the build ran.
            preBuild = ''
              export RUSTFLAGS="''${RUSTFLAGS:-} --remap-path-prefix=$NIX_BUILD_TOP=/build"
              export NIX_CFLAGS_COMPILE="''${NIX_CFLAGS_COMPILE:-} -ffile-prefix-map=$NIX_BUILD_TOP=/build"
            '';
            postInstall = ''
              mkdir -p "$out/share/balthasar"
              cp -r config "$out/share/balthasar/"
            '';

            doInstallCheck = true;
            nativeInstallCheckInputs = [ pkgs.binutils ];
            installCheckPhase = ''
              runHook preInstallCheck

              binary="$out/bin/balthasar"
              home="$TMPDIR/home"
              mkdir -p "$home/run" "$home/tmp"
              chmod 700 "$home/run"
              env -i PATH="$PATH" HOME="$home" TMPDIR="$home/tmp" \
                XDG_CONFIG_HOME="$home/config" XDG_DATA_HOME="$home/data" \
                XDG_CACHE_HOME="$home/cache" XDG_STATE_HOME="$home/state" \
                XDG_RUNTIME_DIR="$home/run" \
                "$binary" verbs --json > "$TMPDIR/verbs.json"
              grep -q '^{"ok":true,"family":1,' "$TMPDIR/verbs.json"
              grep -q '"verb":"serve"' "$TMPDIR/verbs.json"

              env -i HOME="$home" XDG_RUNTIME_DIR="$home/run" "$binary" client \
                | cmp - crates/balthasar-lua/lua/balthasar.lua
              test -s "$out/share/balthasar/config/init.lua"
              test -s "$out/share/balthasar/config/harness/balthasar.lua"
              test "$(ls "$out/bin")" = balthasar

              if readelf -l "$binary" | grep -q 'program interpreter'; then
                echo "error: $binary requests a dynamic loader" >&2
                exit 1
              fi
              if readelf -d "$binary" 2>/dev/null | grep -q NEEDED; then
                echo "error: $binary has dynamic dependencies" >&2
                exit 1
              fi

              runHook postInstallCheck
            '';

            meta = {
              description = "Memory for agents";
              homepage = "https://github.com/ai-nerv/balthasar";
              license = lib.licenses.mit;
              mainProgram = "balthasar";
              platforms = [
                "x86_64-linux"
                "aarch64-linux"
              ];
            };
          };
          app = {
            type = "app";
            program = "${balthasar}/bin/balthasar";
            meta.description = "Run balthasar";
          };
        in
        {
          packages = {
            default = balthasar;
            inherit balthasar;
          };
          apps = {
            default = app;
            balthasar = app;
          };
          checks = {
            inherit balthasar;
          };
        }
      );
    in
    nixpkgs.lib.recursiveUpdate packaged
    (flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Pinned to the version nixpkgs was already giving us, with the musl target added.
        rust = pkgs.rust-bin.stable."1.96.0".default.override {
          extensions = [ "rust-src" ];
          targets = [ "x86_64-unknown-linux-musl" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            rust
            pkgs.rust-analyzer
            pkgs.git-cliff
            pkgs.cargo-machete
            pkgs.cargo-deny
            pkgs.cargo-audit
            pkgs.clang
            pkgs.mold
            pkgs.pkg-config
          ];

          MUSL_CC = pkgs.pkgsMusl.stdenv.cc;
          MUSL_CLANG = pkgs.musl.dev;
        };
      }
    ));
}
