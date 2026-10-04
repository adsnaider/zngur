{
  description = "WASM development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    # Use Nixpkgs' built-in list of all supported systems instead of hardcoding
    systems = nixpkgs.lib.systems.flakeExposed;
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    devShells = forAllSystems (
      system: let
        pkgs = import nixpkgs {inherit system;};

        # Map Nix system architecture to wasi-sdk release naming conventions
        # If the system isn't in the list, it gracefully throws an error.
        wasiSystem =
          {
            "x86_64-linux" = "x86_64-linux";
            "aarch64-linux" = "aarch64-linux";
            "x86_64-darwin" = "x86_64-macos";
            "aarch64-darwin" = "aarch64-macos";
          }.${
            system
          } or (throw "wasi-sdk binaries are not available for your system: ${system}");

        wasi-sdk = pkgs.fetchzip {
          url = "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-30/wasi-sdk-30.0-${wasiSystem}.tar.gz";
          # Update this hash if you change the wasi-sdk version
          hash = "sha256-QXvKCuEO3PKQdjp1R7IecMfabvtwEPd+ANakcVBxrJA=";
        };
        wasmtime-bin = pkgs.stdenv.mkDerivation {
          pname = "wasmtime";
          version = "36.0.2";
          src = pkgs.fetchzip {
            url = "https://github.com/bytecodealliance/wasmtime/releases/download/v36.0.2/wasmtime-v36.0.2-${wasiSystem}.tar.xz";
            hash = "sha256-wZfaP7U2Q4OU86+TnhinUQ2JmwOZm8jduBaOe29Johk=";
          };
          installPhase = ''
            mkdir -p $out/bin
            cp wasmtime $out/bin/
          '';
        };
      in {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cspell
            dprint
            emscripten
            wasmtime-bin

            llvmPackages_latest.lld
          ];

          shellHook = ''
            export WASI_SDK_PATH="${wasi-sdk}"
            export EMSDK_PATH="${pkgs.emscripten}/share/emscripten"
            export RUST_BACKTRACE="0"
          '';
        };
      }
    );
  };
}
