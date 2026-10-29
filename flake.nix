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

          # The prover behind the one gate that must *prove*, not merely
          # parse: `why3 prove -P z3 lib/liquid/golden/vc-fold-premises.mlw`
          # (scripts/verify-invariants, the fold golden). z3 is free and a
          # cache fetch — unlike alt-ergo it cannot take the engine down with
          # it — and it comes from the same channel as why3 above, so driver
          # and prover versions move together. Registration is host state:
          # `why3 config detect` records the absolute path in ~/.why3.conf,
          # after which PATH is irrelevant (an unregistered prover is a gap,
          # not a failure, in that gate).
          pkgsWhy3.z3
        ];

        # GHC environment for building mm0-hs' from-mm path (shared with the
        # mm0-from-mm package below and its devShell).
        #
        # `dynamic-mvector` (a dep of mm0-hs' Compiler.Env, which the from-mm
        # closure reaches through the elaborator) carries an unconditional
        # `broken = true` in hackage-packages.nix with no configuration-nix
        # rationale — a blanket mark, not a recorded build failure. Clear the
        # flag locally; if the build itself fails against vector-0.13 the
        # failure surfaces here instead of at eval time.
        ghcPkgs = pkgs.haskellPackages.override {
          overrides = self: super: {
            dynamic-mvector = super.dynamic-mvector.overrideAttrs (old: {
              meta = (old.meta or { }) // { broken = false; };
              # The actual configure failure: `primitive >=0.5 && <0.7`
              # against primitive-0.9. Nothing else in the package is
              # version-pinned, and the module only uses stable
              # MutVar/MVector APIs, so drop the stale upper bound.
              postPatch = (old.postPatch or "") + ''
                sed -i 's/primitive >=0\.5 && <0\.7/primitive >=0.5/' *.cabal
              '';
            });
          };
        };
        ghcEnv = ghcPkgs.ghcWithPackages (p: with p; [
          mtl binary array data-default utf8-string bytestring containers
          transformers split megaparsec prettyprinter filepath word8 text
          text-builder vector vector-algorithms async async-pool stm hashable
          dynamic-mvector data-reify unordered-containers directory time aeson
        ]);

      in {
        packages.default = pkgs.stdenv.mkDerivation {
          pname = "austral";
          version = "0.2.0";
          src = ./.;
          installFlags = [ "PREFIX=$(out)" ];
          inherit buildInputs;
        };

        # The Why3 engine on its own, installable without the devShell:
        #
        #     nix build .#why3        # or: nix profile install .#why3
        #
        # This exists because the devShell is all-or-nothing: it carries
        # alt-ergo alongside why3, alt-ergo is unfree, and on some hosts its
        # build fails — which then takes the *engine* down with it, even though
        # `why3 prove --type-only` (the gate in scripts/verify-invariants that
        # needs no prover) needs only why3. Splitting the output means the
        # engine is installable by itself, and the prover by itself, rather
        # than both or neither.
        #
        # Same store path as the devShell's entry (`pkgsWhy3.why3`, why3
        # 1.8.2): the version is pinned by this flake, not by the host.
        packages.why3 = pkgsWhy3.why3;

        # The prover `LiquidWhy3.prove` names with `-P alt-ergo`. Separate for
        # the reason above (unfree; its build is the one that fails); install
        # it when the host can build it, and the `Liquid golden Why3 parse
        # check` gap in scripts/verify-invariants closes with it.
        packages.alt-ergo = pkgsWhy3.alt-ergo;

        # The frozen MM0 toolchain — the Phase-D root of trust
        # (docs/lean4mm0.md §5, CI `phase-d` job): `mm0-rs` elaborates the
        # generated `.mm1` proofs into an `.mmb`, and `mm0-c` — one C file,
        # no dependencies — re-verifies that binary against the `.mm0` spec.
        # Pinned to the *same* digama0/mm0 commit the CI job checks out, so
        # the local `scripts/lean2mm0 verify` and the CI gate run byte-identical
        # verifiers:
        #
        #     nix build .#mm0          # or: nix profile install .#mm0
        #
        # mm0-rs builds with cargo's default features exactly like CI's
        # `cargo install --path mm0-rs`; mm0-c is compiled with the same
        # one-line gcc invocation CI uses (mm0-c/make.sh's non-PGO variant).
        packages.mm0 = pkgs.rustPlatform.buildRustPackage {
          pname = "mm0";
          version = "0.1.0-unstable-2026-09-30";
          src = pkgs.fetchFromGitHub {
            owner = "digama0";
            repo = "mm0";
            rev = "0d414c0bfdaaeb7fea571895127abc1fa5a3d956";
            hash = "sha256-EIzcXTX6bqhq/IRCDRvHcuXDuKashy+a1p7taUsunBw=";
          };
          # Cargo.toml/Cargo.lock live in mm0-rs/ (a workspace root); the
          # build and the vendored deps both root there.
          cargoRoot = "mm0-rs";
          buildAndTestSubdir = "mm0-rs";
          cargoHash = "sha256-LgqCWvQHPbV/sa5F37kbi3F4DRDewN0ifdAyoBL7ig8=";
          postBuild = ''
            gcc mm0-c/main.c -O2 -o mm0-c-verifier
          '';
          postInstall = ''
            install -Dm755 mm0-c-verifier $out/bin/mm0-c
          '';
        };

        # `from-mm` — the Metamath → MM0/MMB translator from the mm0 repo
        # (mm0-hs, digama0/mm0, same pinned commit as packages.mm0), built
        # with only the from-mm dependency closure: the LSP server modules
        # (haskell-lsp, lens, hslogger) are unreachable from a from-mm-only
        # driver, so the ancient lsp stack is not needed. One derivation,
        # pinned sources, no network at build time:
        #
        #     nix build .#mm0-from-mm
        #
        packages.mm0-from-mm =
          let
            mm0Src = pkgs.fetchFromGitHub {
              owner = "digama0";
              repo = "mm0";
              rev = "0d414c0bfdaaeb7fea571895127abc1fa5a3d956";
              hash = "sha256-EIzcXTX6bqhq/IRCDRvHcuXDuKashy+a1p7taUsunBw=";
            };
          in pkgs.stdenv.mkDerivation {
            pname = "mm0-from-mm";
            version = "0.1.0-unstable-2026-09-30";
            src = mm0Src;
            sourceRoot = "source/mm0-hs";
            nativeBuildInputs = [ ghcEnv pkgs.alex pkgs.happy ];
            # A from-mm-only driver: the stock Main.hs imports MM0.Server
            # (LSP), which this build deliberately excludes.
            #
            # mm0-hs targets the GHC/text/mtl era it was written for; this
            # patch ports it to the toolchain above (mtl 2.3 no longer
            # re-exports Control.Monad, text dropped Text.Builder,
            # hashable added an Eq superclass, plus the four default-
            # extensions package.yaml used to supply). Verified by building
            # the same tree with ghc --make (see devShells.mm0hs).
            buildPhase = ''
              runHook preBuild
              patch -p1 < ${./mm0/from-mm-compat.patch}
              cat > Main.hs <<'EOF'
              module Main (main) where
              import System.Environment
              import System.Exit
              import MM0.FromMM
              main :: IO ()
              main = getArgs >>= \case
                "from-mm" : rest -> fromMM rest
                "show-bundled" : rest -> showBundled rest
                _ -> die "mm0-from-mm: usage: mm0-from-mm from-mm MM-FILE [-o MM0-FILE MMU/MMB-FILE]"
              EOF
              ghc -O2 --make -isrc -outputdir .ghc-out \
                -XFlexibleContexts -XLambdaCase -XOverloadedStrings -XTupleSections \
                Main.hs -o mm0-from-mm
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              install -Dm755 mm0-from-mm $out/bin/mm0-from-mm
              runHook postInstall
            '';
          };

        devShells.default = pkgs.mkShell {
          inherit buildInputs;
        };

        # Shell for iterating on the from-mm build: nix develop .#mm0hs
        devShells.mm0hs = pkgs.mkShell {
          packages = [ ghcEnv pkgs.alex pkgs.happy ];
        };
      });
}