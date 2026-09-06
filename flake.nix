{
  description = "balthasar Rust development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=4c1018dae018162ec878d42fec712642d214fdfa";
    flake-utils.url = "github:numtide/flake-utils";
    nixgl.url = "github:nix-community/nixGL";
    # For the musl target's standard library, and nothing else.
    #
    # nixpkgs ships `rustc` with std for the host only, so `--target x86_64-unknown-linux-musl`
    # fails on "can't find crate for `core`". Its answer is `pkgsCross.musl64.rustc`, which
    # compiles a whole cross rustc from source -- hours, for a file we already have. This
    # overlay hands over the official prebuilt std for a named target instead.
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    { nixpkgs, flake-utils, nixgl, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [
          rust-overlay.overlays.default
          (final: prev: {
            xorg = prev.xorg // {
              libX11 = final.libx11;
              libxcb = final.libxcb;
              libxshmfence = final.libxshmfence;
            };
          })
        ];

        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
            nvidia.acceptLicense = true;
          };
        };

        # Pinned to the version nixpkgs was already giving us, with the musl target added.
        # Floating to `latest` would drift the compiler underneath a workspace that builds
        # under `-Dwarnings`, where one new lint is a failed verify.
        rust = pkgs.rust-bin.stable."1.94.0".default.override {
          extensions = [ "rust-src" ];
          targets = [ "x86_64-unknown-linux-musl" ];
        };

        nvidiaVersion = builtins.getEnv "NVIDIA_VERSION";
        hasNvidia = nvidiaVersion != "";

        nixglPkgs = import "${nixgl}/default.nix" ({
          inherit pkgs;
        } // pkgs.lib.optionalAttrs hasNvidia {
          inherit nvidiaVersion;
          nvidiaHash = null;
        });

        nixGLTarget =
          if hasNvidia
          then "${nixglPkgs.nixGLNvidia}/bin/nixGLNvidia-${nvidiaVersion}"
          else "${nixglPkgs.nixGLIntel}/bin/nixGLIntel";
        nixVulkanTarget =
          if hasNvidia
          then "${nixglPkgs.nixVulkanNvidia}/bin/nixVulkanNvidia-${nvidiaVersion}"
          else "${nixglPkgs.nixVulkanIntel}/bin/nixVulkanIntel";

        nixGLAlias = pkgs.runCommand "nixGL" { } ''
          mkdir -p $out/bin
          ln -s ${nixGLTarget} $out/bin/nixGL
        '';
        nixVulkanAlias = pkgs.runCommand "nixVulkan" { } ''
          mkdir -p $out/bin
          ln -s ${nixVulkanTarget} $out/bin/nixVulkan
        '';

        guiLibs = with pkgs; [
          alsa-lib
          udev
          vulkan-loader
          libxkbcommon
          wayland
          libx11
          libxcursor
          libxi
          libxrandr
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            # One toolchain, carrying cargo, rustfmt and clippy with it. Listing nixpkgs's
            # rustc beside this one puts two compilers on PATH and the first wins by accident.
            rust
            pkgs.rust-analyzer
            pkgs.git-cliff
            # Reads every manifest against its own sources, so a dependency nothing uses is a
            # failure rather than a line somebody eventually notices. See the note in
            # `Cargo.toml` for why this and not `unused_crate_dependencies`.
            pkgs.cargo-machete
            pkgs.clang
            pkgs.mold
            pkgs.pkg-config

          # Graphics wrappers and the libraries a windowed program needs, on x86_64 only.
          #
          # nixGL pulls `pkgsi686Linux` for its 32-bit loaders, which does not exist on aarch64 —
          # so a shell that asked for it unconditionally could not be *evaluated* there at all,
          # let alone entered. That made `nix develop` fail on the arm64 release runner with
          # "i686 Linux package set can only be used with the x86 family", which took the amd64
          # leg down with it and produced no binary for either.
          #
          # None of it is needed to build: this is for running something that draws, on a
          # machine whose driver needs wrapping.
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isx86_64 ([
            nixGLAlias
            nixVulkanAlias
            nixglPkgs.nixGLIntel
            nixglPkgs.nixVulkanIntel
          ] ++ pkgs.lib.optionals hasNvidia [
            nixglPkgs.nixGLNvidia
            nixglPkgs.nixVulkanNvidia
          ] ++ guiLibs);

          # A musl toolchain for the static build, handed over as a path rather than a package.
          # As a package its headers land on the default search path, and an ordinary build then
          # compiles against musl while linking against glibc -- which succeeds without a word and
          # crashes at startup. Only the static build is given it: .make.lua reads MUSL_CC.
          # gcc targeting musl, which is the only one of the two that has a C++ standard library.
          MUSL_CC = pkgs.pkgsMusl.stdenv.cc;
          # musl-clang: the host clang, pointed at musl's headers and libs. C only -- it has no
          # libstdc++, so a C++ build against it fails on the first #include <string>.
          MUSL_CLANG = pkgs.musl.dev;

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibs;
          WGPU_VALIDATION = "0";
          WGPU_DEBUG = "0";
        };
      }
    );
}
