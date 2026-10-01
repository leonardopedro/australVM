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
        # Why3 toolchain, imported from this same channel so `pkgs.why3` is the
        # *same store path* ../unfer resolves (see the note on the input below).
        # `allowUnfree` is scoped to this import and only because alt-ergo is
        # unfree in nixpkgs — unfer already sets it repo-wide for CUDA, so the two
        # projects use the same prover rather than one real and one free variant.
        pkgsWhy3 = import nixpkgs {
          system = system;
          config.allowUnfree = true;
        };
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

          # Why3 verification engine (docs/LIQUID.md §2.3).
          #
          # SHARED TOOLCHAIN with ../unfer (S36 there, L5/L10 here). Both repos
          # run `why3 prove` as a subprocess over `.mlw` files, so they must be
          # the same *version*, not merely the same package: the two repos'
          # stdlib assumptions differ between releases, and `lib/why3_plugin/
          # unfer_ocaml.drv` is a concrete case — it maps `bool.Bool`'s `=`,
          # which exists in 1.8.x and does not in 1.6.0, so extraction fails on
          # the older engine while still exiting 0.
          #
          # This side is on nixos-unstable (why3 1.8.2); unfer takes that same
          # channel for `why3`/`alt-ergo` only, since its base channel is
          # 23.05 for CUDA. One build, one version, one `~/.why3.conf`.
          # **Changing the version here means changing it there.**
          #
          # LiquidWhy3 shells out to `why3 prove`, and before this the engine was
          # simply absent, so `LiquidWhy3.prove` took its structured-skip path
          # and every emitted `.mlw` went unchecked. That is the safe direction
          # — under-claiming leaves an obligation to prove later — but it also
          # means a *wrong* `.mlw` was indistinguishable from an unproved one:
          # nothing could tell "the prover said no" from "there was no prover".
          # Having the engine is what let L10's emitted theory be validated
          # rather than pinned as a golden and hoped over.
          pkgsWhy3.why3

          # The prover `LiquidWhy3.prove` asks for. `why3` alone is not enough:
          # `why3 prove -P alt-ergo` resolves the prover through Why3's own
          # config (~/.why3.conf), which maps the *name* to a binary on PATH.
          # Without the binary on PATH the invocation reports "No prover
          # corresponds to alt-ergo" and — note — still exits 0, so a missing
          # prover looks exactly like a successful one to any caller that only
          # checks the exit status.
          pkgsWhy3.alt-ergo
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