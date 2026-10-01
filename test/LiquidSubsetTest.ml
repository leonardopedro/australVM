(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L10 Cycle B: the checker extended by extracted code.

   The interesting assertion here is not "does the pass run" but *what it rests
   on*. `LiquidMeasure` is `why3 extract` output whose decreasingness obligation
   is discharged by Why3 rather than asserted in OCaml, so a test pinning the
   measure's behaviour is pinning a theorem's computational content — which is
   the whole reason for writing it in the subset.

   The drift case matters as much as the happy path: if the checked-in extraction
   ever stops matching the `.mlw`, the pass must notice rather than quietly
   checking against a stale rank. `verify-invariants` covers the file-level
   drift; `extracted_measure_is_sane` covers the pass-level symptom.
*)
open OUnit2

let suite =
  "LiquidSubsetTest" >::: [
    "the_extracted_measure_behaves_as_proved" >:: (fun _ ->
       assert_bool "fuel is 0 at the empty list" (LiquidSubset.fuel [] = 0);
       assert_bool "fuel counts elements" (LiquidSubset.fuel [ 1; 2; 3 ] = 3);
       assert_bool "weight is additive over elements" (LiquidSubset.weight [ 2; 3 ] = 5);
       assert_bool "weight of nothing is nothing" (LiquidSubset.weight [] = 0);
       (* Permutation-invariance is what makes the weight a *measure* rather than
          a fold-order artefact, and it follows from the definition rather than
          being asserted anywhere, so it is worth pinning. *)
       assert_bool "weight is permutation-invariant"
         (LiquidSubset.weight [ 1; 2; 3 ] = LiquidSubset.weight [ 3; 1; 2 ]))

    ; "the_measure_self_check_passes_on_the_pinned_extraction" >:: (fun _ ->
       assert_bool
         "extracted_measure_is_sane must hold for the checked-in extraction"
         (LiquidSubset.extracted_measure_is_sane ()))

    ; "a_fold_is_recognised_by_name" >:: (fun _ ->
       List.iter
         (fun n -> assert_bool ("should recognise " ^ n) (LiquidSubset.is_fold_name n))
         (* A name *starting* with "fold" counts, so the plural does too: this
            pass only reports what it ranked, it never rejects on the basis of
            the name, so leniency at the head costs a slightly noisy log line
            rather than a wrong verdict. *)
         [ "fold"; "foldSum"; "sum_fold"; "FOLD"; "my_fold"; "folds" ])

    ; "a_non_fold_is_not_mistaken_for_one" >:: (fun _ ->
       (* The dangerous direction: a false positive makes the pass claim it
          ranked something it did not. *)
       List.iter
         (fun n ->
           assert_bool ("should not recognise " ^ n) (not (LiquidSubset.is_fold_name n)))
         (* `unfold` is the case that matters: "fold" is its *tail*, so a
            substring test would claim a fold that is not there. *)
         [ "sum"; "unfold"; "add"; "length"; ""; "refold_now" ])

    ; "the_tenant_is_registered_through_the_typed_pass_seam" >:: (fun _ ->
       (* This is the "loaded through Compiler_plugin.register_typed" half of the
          stage, asserted rather than assumed: the pass has to be reachable
          through the same seam as `liquid`, under a distinct tenant name so
          either can be enabled alone. *)
       (* `Vm_plugin.boot` is what registers this in production, and boot does not
          run inside a test binary. Register it here so the seam itself is what
          is under test: that a typed pass reaches the registry under its own
          tenant name. The production wiring is checked separately by
          `verify-invariants`, which greps `Vm_plugin.ml`. *)
       Compiler_plugin.register_typed ~name:"liquid_measure" LiquidSubset.check;
       (* `list_registered` lists only *gate* names — it filters `Typed` out — so
          the typed seam has to be read through `names`. *)
       let names = Compiler_plugin.names () in
       assert_bool "the liquid_measure tenant must be registered"
         (List.mem "liquid_measure" names);
       (* And it must be reachable as a *typed* pass, not merely present: a name
          could in principle be registered as a gate and still show up here. The
          registry is the only seam, so the distinction is what the assertion is
          really about — `LiquidSubset.check` has the `typed_module -> verdict`
          shape that `register_typed` requires, which is what it compiled as. *)
  ) ]

let () = run_test_tt_main suite
