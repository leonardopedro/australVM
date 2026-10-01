{
  description = "Systems language with linear types and capability-based security.";

  # nixos-unstable: the cranelift JIT bridge (cranelift 0.131) needs a rustc
  # newer than nixos-23.05 ships, and the bridge .so / OCaml toolchain must
  # share a glibc so OCaml test binaries can load the .so at runtime. The
  # 23.05 pin (glibc 2.37) cannot load a .so built on a modern host.
  # velysterm/unfer's flakes follow the same nixos-unstable convention.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Why3 toolchain channel.
  #
  # Pinned to the SAME nixpkgs revision ../unfer uses (its `nixpkgs` input, the
  # nixos-23.05 channel at rev 70bdade…). That is deliberate: `pkgs.why3` is
  # then the *same store path* in both repositories, so there is one Why3 build
  # on disk rather than two, and one version rather than two.
  #
  # This matters beyond disk. Before this, australVM resolved why3 1.8.2 from
  # nixos-unstable and unfer resolved why3 1.6.0 from nixos-23.05 — two builds,
  # two versions, and two different `.why3.conf` prover registrations, with a
  # `.mlw` from australVM's emitter checked by one engine and unfer's WhyML
  # checked by another. A prover that accepts a file in one version and rejects
  # it in the other would have shown up as a mysterious refusal in one project
  # only. (The emitted goldens were confirmed to parse under both versions.)
  #
  # Only the Why3 toolchain comes from this channel. australVM's own build inputs
  # stay on nixos-unstable, because the cranelift-bridge/glibc constraint in the
  # note above rules out 23.05 for the OCaml and Rust toolchains — moving the
  # base channel would break the bridge to buy nothing.
  inputs.why3-nixpkgs.url = "github:NixOS/nixpkgs/70bdadeb94ffc8806c0570eb5c2695ad29f0e421";

  inputs.utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, why3-nixpkgs, utils }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # `allowUnfree` only for this import, and only because alt-ergo is
        # unfree in nixpkgs. ../unfer already sets it repo-wide for CUDA, so the
        # two projects agree on which prover they use: `alt-ergo` proper, not a
        # free variant that would be a *different* binary and a second thing to
        # keep in sync.
        pkgsWhy3 = import why3-nixpkgs {
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

          # Why3 verification engine (docs/LIQUID.md §2.3), taken from unfer's
          # channel so both repositories share one build — see the
          # `why3-nixpkgs` input above.
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