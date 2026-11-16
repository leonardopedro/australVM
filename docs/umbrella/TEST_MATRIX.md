# Umbrella production acceptance matrix

**Proposed tests, not current passing coverage.** See the
[plan](../../PLAN_umbrella_production.md) for execution order and the
[review](../UMBRELLA_PRODUCTION_REVIEW.md) for current defects. Every case must
run against the actual production helper or binary once implemented. Extracted
source probes and fake tools are useful for isolation but cannot replace a real
end-to-end lane.

## Evidence rules

Each run records case ID, input/fixture digest, tested binary/toolchain identity,
command, exit/signal, timeout, bounded stdout/stderr, external oracle and cleanup
result. A passing negative case names the exact refusal and checks that its
observable effect did not occur. “Some error happened” is insufficient.

Use private directories and loopback-only test servers with temporary identities.
Outside-root sentinel files contain unique unpredictable marker bytes. Test
valid pointer lifetimes in-process; invalid-pointer, abort, signal and UB probes
run in disposable children with deadlines, never in the agent process. The
normal test suite never invokes arbitrary remote crates or public services.

Portable unit lanes must have no silent skips. Hardware/toolchain-dependent
lanes can report an explained gap during development; a profile requiring that
lane cannot be released while it is a gap. A capability test compiled without
the `capstd` feature is not capability enforcement coverage.

## Harness and CLI

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| HAR01 | Run an explicit case ID and lane | Exactly requested cases execute; zero selection is an error | U00 |
| HAR02 | Test executable prints PASS but performs no operation | Runner rejects missing marker/oracle; exit 0 alone cannot satisfy case | U01 |
| HAR03 | Required feature/tool absent | Explained gap; strict required lane fails | U00 |
| HAR04 | Mutate a test control into a no-op | Mutation runner detects unapplied/irrelevant mutation | U24 |
| CLI01 | Help/version anywhere allowed by CLI grammar | Exit 0, stable usage/version, no build/load marker | U03 |
| CLI02 | Unknown/prefix option, missing or empty required value | Exit 2, named usage diagnostic, no child spawned | U03 |
| CLI03 | `--entry --expect 7`, malformed integer, duplicate option | Defined grammar/refusal; no accidental flag consumption | U03 |
| CLI04 | Paths with spaces/non-UTF-8 and positional `--` | Exact OS path reaches tool; supported escaping preserves arguments | U03 |
| CLI05 | Valid `--k=v`, negative expected i64, legacy demo syntax | Same parsed contract as separated form, unambiguous negatives | U03 |
| CLI06 | JSON mode with child noise and runtime refusal | One typed event per line; stdout contains only protocol; no success after failure | U03/U15 |

## Build orchestration and CPS

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| BLD01 | Dependency proc-macro shared library appears before target cdylib | Exact requested package/target artifact selected | U05 |
| BLD02 | Workspace exposes two cdylibs | Explicit selection works; ambiguous default refuses | U05 |
| BLD03 | Missing artifact, malformed JSON, build-finished false, failed compiler | Structured build failure; no native load | U05 |
| BLD04 | Custom target dir, release/debug, features and no-default-features | Report and select the actual requested artifact/configuration | U05 |
| BLD05 | Foreign architecture/lib format passed to native run | Refuse before dlopen/execute; no crash | U04/U27 |
| BLD06 | Build script/proc macro creates a marker | Runs only under authorized build-trust profile; call grants do not claim containment | U02/U26 |
| BLD07 | Child hangs, forks a child, floods both pipes or is signaled | Deadline/size bound; reap owned descendants; useful partial diagnostics | U06 |
| BLD08 | Missing Cargo/rustc/frontend/linker/bridge or mismatched version | Specific prerequisite error with stage identity | U00/U25 |
| BLD09 | Offline locked build with unavailable dependency; lock mutation required | Explicit refusal; lockfile remains unchanged | U05/U25 |
| CPS01 | Frontend fails or succeeds without expected CPS output | Refuse missing/stale artifact, preserve diagnostic stage | U07 |
| CPS02 | Pre-existing CPS symlink/stale file; frontend crash | Private output cannot overwrite sentinel or consume stale bytes | U06/U07 |
| CPS03 | Library + client modules, interface/body pair | Bundle includes all definitions exactly once; call returns independent expected value | U17 |
| CPS04 | Same short import/export name in two source modules | Qualified resolution or explicit collision error; no first-wins misbinding | U08/U17 |
| CPS05 | Unsupported CPS instruction, malformed lengths, huge/deep input | Bounded refusal, no C fallback claimed as JIT success | U07/U24 |
| CPS06 | Compile/check command on effectful fixture | Zero target invocation and zero native initialization during compile-only stage | U07 |
| CPS07 | Required liquid/totality failure or absent prover | Strict build refuses; optional build records exact uncovered requirement | U07/U28 |

## ABI, symbols and execution

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| ABI01 | No-arg i64 entry returns 0, -1, i64::MIN, i64::MAX | Values succeed independently of error status | U04/U15 |
| ABI02 | Empty, missing, private, ambiguous or parameterized entry | Exact safe refusal before invocation | U04 |
| ABI03 | Wrong arity/type/return/descriptor version or provider digest | Pre-execution mismatch, no native call marker | U10 |
| ABI04 | Signed/unsigned widths, boolean tags, checked narrowing | Direct Rust oracle agrees or unsupported signature refuses | U11 |
| ABI05 | Float/mixed float-int/NaN/infinity/-0 | Specified bit encoding or correctly typed trampoline agrees with direct call | U11 |
| ABI06 | Struct/enum/ZST/Option/Result/unit return | Versioned adapter representation; unsupported direct layout refuses | U10/U15/U21 |
| ABI07 | Same descriptor on aarch64 and x86_64 | Target-specific expectations checked; foreign ABI never silently reused | U27 |
| ABI08 | Rust panics inside actual cdylib `extern C` wrapper | Wrapper translates unwind panic; abort profile is a child failure, never fake success | U15/U23 |
| LNK01 | Two crates export identical rs name; reversed load order | Explicit provider binding or deterministic rejection | U08 |
| LNK02 | Granted name supplied by wrong/preloaded provider | Artifact/ABI identity mismatch; no attacker marker | U08/U10 |
| LNK03 | Non-rs arbitrary import or reserved au/__ name | Strict profile rejects undeclared provider/import | U08/U09 |
| LNK04 | Static registration after JIT init or address replacement | Defined failure; existing instances stay bound to original provider | U08/U18 |
| LNK05 | Drop provider while JIT/owned handles remain | Mapping retained safely until last dependent generation is retired | U13/U18 |
| LNK06 | Repeated run/reset/load failure | No stale last-error, provider table or wrong entry selection | U15/U18 |

## Ownership, borrows and richer Rust APIs

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| OWN01 | Null/zero/negative/oversize buffer length | Specified empty/null convention; checked refusal before slice construction | U12 |
| OWN02 | Embedded NUL, invalid UTF-8, binary and empty string | Byte/text APIs distinguish representations and validate UTF-8 | U12 |
| OWN03 | Rust-owned value constructed, moved and destroyed | Exactly one observed Drop in originating allocator/library | U13 |
| OWN04 | Copy/use-after-move/wrong-type/foreign-instance handle | Austral negative fixture or runtime rejection, no second Drop | U13 |
| OWN05 | Reused handle slot, exhausted generation, double destroy | Stale handle refused; no ABA reuse or wrong object destruction | U13 |
| OWN06 | Borrowed child retained after owner replace/drop | Compile rejection or generation-checked runtime refusal | U14 |
| OWN07 | Overlapping mutable borrows or reentrant mutation | No simultaneous Rust `&mut` aliases; exact refusal/serialization rule | U14/U16 |
| OWN08 | Panic/early return/error during ownership transfer | Defined moved/not-moved state; destructor counter matches contract | U13/U15 |
| OWN09 | Rust allocation freed by wrong allocator or old library generation | Unsupported crossing prevented; correct destroy thunk used | U13/U18 |
| API01 | Free function and inherent method through generated adapter | Real crate call equals direct Rust oracle | U21 |
| API02 | Concrete generic instantiations and two crate versions | Separate qualified descriptors; Rust checks trait bounds | U17/U21 |
| API03 | Unsupported trait object/lifetime/variadic/async form | Actionable diagnostic; no guessed layout or erased bound | U10/U21 |
| API04 | Synchronous callback on valid call scope | Arguments/results correct; callback grant/context retained | U16 |
| API05 | Callback stored/called after unregister or owner drop | Invalid token refused, no jump to retired JIT memory | U16/U18 |
| API06 | Callback recursion/thread hop without permission | Bound/deadlock-safe refusal; Send/Sync enforced | U16/U23 |
| API07 | Callback or destructor panics | Error contained at own boundary; cleanup reported, worker remains controlled | U15/U16 |
| API08 | Optional async future cancelled/restarted/completed twice | Stable operation identity and explicit pending/completed ownership | U31 |

## Authorization and capability IO

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| AUT01 | Missing/malformed manifest, unknown strict fields, unavailable profile | Refuse before Cargo/native load when no trusted build/load authorized | U02 |
| AUT02 | rs grant absent or only uk grant present | UK-4001 where applicable and zero rs call marker | U09 |
| AUT03 | Two modules/instances with disjoint grants | A never uses B's caller identity or handles | U09 |
| AUT04 | Same-name policy narrows; failed load/swap | Old permissions do not reappear from global merge; failed load preserves prior context | U09/U18 |
| AUT05 | Changed provider/library/ABI/policy after validation | Bound artifact unchanged or refused; validate/use race covered | U08/U25 |
| AUT06 | Feature capstd disabled with fs/net requirements | Unsupported profile refuses; no fake capability pass | U19/U28 |
| FS01 | Granted root read with outside-root marker present | Read exact inside bytes; missing grant cannot read same file | U19 |
| FS02 | `..`, absolute path, symlink to outside sentinel | Outside read/write refused; sentinel bytes unchanged | U19 |
| FS03 | Create/rename/link/remove through symlinked parent and rename race | Outside sentinel remains unchanged; operation obeys descriptor-relative rules | U19 |
| FS04 | Read-only vs write/create/delete/list grants | Each forbidden operation refused; no implied broader rights | U19 |
| FS05 | Second module attempts first module's directory/file handle | Ownership/generation refusal | U19/U09 |
| FS06 | Non-UTF-8 names, broken symlink, denied path, absent path | Preserve supported names; distinguish error from not-found | U19 |
| FS07 | Removed/renamed root, revoked capability, open handle after revocation | Documented descriptor/policy semantics enforced | U19/U18 |
| FS08 | Read/write exceeds quota or output buffer; partial IO | Bounded allocation and explicit partial/error result | U19/U23 |
| NET01 | Granted loopback endpoint vs denied endpoint | Allowed response observed; denied server accepts zero connection | U20 |
| NET02 | DNS answer/re-resolution changes to denied address | Check actual dial target under policy; denied marker untouched | U20 |
| NET03 | HTTP redirect/proxy/socket reuse crosses authority | Policy reapplied at new destination; no unauthorized dial | U20 |
| NET04 | TLS wrong name/cert, peer closes, connect/read timeout | Typed refusal/failure; no insecure fallback | U20 |
| NET05 | Capability revoked on reconnect or another instance reuses socket token | Current owner/policy enforced per declared lifetime | U20/U18 |
| NET06 | Unsupported UDP/listen/raw-socket feature requested | Explicit refusal; no implicit ambient networking | U20 |

## Isolation, packaging and release behavior

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| RUN01 | Native infinite loop, abort, segfault or arithmetic trap | Parent detects signal/deadline, reaps worker, returns non-success | U23 |
| RUN02 | Worker memory/process/fd/output exhaustion | Enforced configured limit or unsupported profile; parent remains responsive | U23/U26 |
| RUN03 | Run/Cargo/frontend cancellation at each stage | Owned children stopped; result classified; private scratch cleaned or retained by policy | U06/U23 |
| RUN04 | Native `std::fs`/raw socket escape attempt | Trusted profile documents bypass; untrusted profile must block it | U26 |
| RUN05 | Build script/proc macro escape attempt | Tested build-isolation policy; runtime isolation does not count as build isolation | U26 |
| RUN06 | Library initializer creates marker or blocks | Authorized load only; isolated worker deadline; denial claim scoped correctly | U02/U23 |
| RUN07 | Warm cache, source/dependency/feature/target/policy edit | Correct rebuild/invalidation; cached code never caches authorization | U18/U25 |
| RUN08 | Concurrent invocations with same project name | Independent scratch/output identities; no interleaved partial artifacts | U06/U18 |
| PKG01 | Install into new prefix and remove checkout from lookup | CLI/build/run work with declared toolchain and runtime files | U25/U27 |
| PKG02 | Wrong frontend/bridge/rustc/ABI schema combination | Version handshake rejects with specific identities | U25 |
| PKG03 | Fresh locked build in different roots | Normalized manifest identity stable within declared reproducibility scope | U25 |
| PKG04 | Unsupported OS/architecture/cross-run | Early supported-platform diagnostic; no native load attempt | U27 |
| PKG05 | Repeated worker generations/reload | Bounded fd/RSS/temp growth under measured budget; no old callback execution | U18/U30 |
| PKG06 | Strict release runner omits a case or hides a skip | Coverage/required-case gate fails | U28/U29 |
| PKG07 | RUSTC_WRAPPER/RUSTFLAGS, Cargo config or loader environment substitutes a tool/dependency | Declared override policy refuses or records a new build/provider identity; no silent substitution | U05/U06/U25 |

## Optional compiler-oracle extension

These cases gate only the direct-Rust-import feature. Stable generated adapters
do not require the fork or Valen to build.

| ID | Trigger | Required oracle | Task |
|---|---|---|---|
| ORC01 | Wrong rustc commit/sysroot/oracle schema | Refuse before loading private compiler APIs | U32 |
| ORC02 | Qualified item, method, concrete generic and Drop query | Matches rustc and stable adapter's direct-call oracle | U32 |
| ORC03 | Unsupported ABI pass mode/trait/lifetime/type | Typed unsupported result; no panic/guess | U32 |
| ORC04 | Cold/warm build with changed imported item | Complete dependency/mono list and correct invalidation every time | U32 |
| ORC05 | Placeholder body substituted or extra module linked | No placeholder-derived optimizer assumptions; retained required symbols | U32 |
| ORC06 | Borrow invalidation example; borrow-check disabled | Invalid lifetime rejected; disabled checker cannot enter production profile | U32 |

## Required negative controls

Each control runs in a disposable worktree/copy and restores the tested sources.
The runner verifies the intended edit applied and verifies the original case
passed first. A changed test expectation is not an enforcement control.

| Mutation | Tests that must detect it |
|---|---|
| Restore first-shared-library selection | BLD01/BLD02 |
| Skip ABI/provider/signature comparison | ABI03, LNK02 |
| Treat ownership handle as freely reusable integer | OWN04/OWN05 |
| Skip borrow invalidation | OWN06/OWN07 |
| Replace capability denial with ambient filesystem/network call | FS01/FS02/FS04, NET01 |
| Skip native call authorization | AUT02/AUT03 |
| Replace result status with raw i64/success | ABI01/ABI08, CLI06 |
| Disable worker deadline or child cleanup | BLD07, RUN01/RUN03 |
| Reuse stale CPS output/cache policy verdict | CPS01/CPS02, RUN07 |
| Print success without executing demo operations | HAR02, FS01, NET01 |

Full release requires the declared profile's cases and controls. Optional cases
remain explicitly unsupported until their implementations and evidence exist.
