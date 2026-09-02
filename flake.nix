{
  description = "Systems language with linear types and capability-based security.";

  # nixos-unstable: the cranelift JIT bridge (cranelift 0.131) needs a rustc
  # newer than nixos-23.05 ships, and the bridge .so / OCaml toolchain must
  # share a glibc so OCaml test binaries can load the .so at runtime. The
  # 23.05 pin (glibc 2.37) cannot load a .so built on a modern host.
  # velysterm/unfer's flakes follow the same nixos-unstable convention.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, utils }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        buildInputs = with pkgs; [
          # General
          gmp
          python311

          # Tooling
          ocamlPackages.ocaml
          ocamlPackages.dune_3
          ocamlPackages.findlib
          ocamlPackages.odoc

          # Rust toolchain for the cranelift JIT bridge ("make bridge"
          # rebuilds safestos/cranelift against this environment's glibc).
          rustc
          cargo

          # OCaml libraries
          ocamlPackages.yojson
          ocamlPackages.ppx_deriving
          ocamlPackages.ounit2
          ocamlPackages.menhir
          ocamlPackages.sexplib
          ocamlPackages.ppx_sexp_conv
          ocamlPackages.zarith
        ];

      in {
        packages.default = pkgs.stdenv.mkDerivation {
          pname = "austral";
          version = "0.2.0";
          src = ./.;
          installFlags = [ "PREFIX=$(out)" ];
          inherit buildInputs;
        };

        devShells.default = pkgs.mkShell {
          inherit buildInputs;
        };
      });
}