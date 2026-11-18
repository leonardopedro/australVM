//! Hermetic check of `stdlib/pkg/Bootstrap.aum`: the four seed recipes carry
//! one real, content-addressed source cell each, and that identity round-trips
//! through the store.
//!
//! Hermetic means *no network, no compiler, no pre-populated store*: the
//! module text is baked in with `include_str!`, and the payloads are
//! generated here. That can pin the *shape* of the pinned digests and the
//! store contract they rely on (an identity is name + version + content, so
//! re-ingesting reproduces it and a different payload cannot satisfy it),
//! but it cannot recompute a digest that names a downloaded tarball — that
//! round trip needs the payloads, and it is the `#[ignore]`d acceptance
//! below, run against `target/pkg-store` as `PLAN_avm_pkg.md` §3 describes.

use avm_cas::engine::unhex;
use avm_cas::ChunkStore;

const BOOTSTRAP: &str = include_str!("../../../../stdlib/pkg/Bootstrap.aum");

/// recipe → (cell name, cell version, tarball). The store cell is the upstream
/// *package* cell that was ingested, so `bootstrap_seed`'s source is the tcc
/// 0.9.27 cell — the derivation's own name/version live in the recipe, not in
/// the payload's manifest.
const PAYLOADS: [(&str, &str, &str, &str); 4] = [
    ("bootstrap_seed", "tcc", "0.9.27", "tcc-0.9.27.tar.bz2"),
    ("musl", "musl", "1.2.5", "musl-1.2.5.tar.gz"),
    ("busybox", "busybox", "1.36.1", "busybox-1.36.1.tar.bz2"),
    ("hello", "hello", "2.12", "hello-2.12.tar.gz"),
];

/// One `constant <x>: Derivation := Derivation(...)` literal.
#[derive(Debug, PartialEq, Eq)]
struct Recipe {
    name: String,
    version: String,
    sources: Vec<String>,
}

/// Parse the module's literals by hand: the file is a fixed set of record
/// constructors, and a real Austral parse would add a dependency this test
/// does not need. Anything unexpected is a test failure, not a skip.
fn recipes(src: &str) -> Vec<Recipe> {
    let body = src
        .split("module body Bootstrap is")
        .nth(1)
        .expect("Bootstrap module body");
    let mut out = Vec::new();
    for chunk in body.split("constant ").skip(1) {
        let quoted = |field: &str| -> String {
            let at = chunk
                .find(field)
                .unwrap_or_else(|| panic!("recipe without {field}: {chunk}"));
            let rest = &chunk[at + field.len()..];
            let start = rest.find('"').expect("opening quote") + 1;
            let end = rest[start..].find('"').expect("closing quote") + start;
            rest[start..end].to_string()
        };
        let sources_at = chunk.find("sources =>").expect("sources field");
        let deps_at = chunk
            .find("dependencies =>")
            .expect("dependencies field follows sources");
        let sources = chunk[sources_at..deps_at]
            .split("head =>")
            .skip(1)
            .map(|s| {
                let s = s.trim_start();
                let s = s.strip_prefix('"').expect("source is a string literal");
                s[..s.find('"').expect("closing quote")].to_string()
            })
            .collect();
        out.push(Recipe {
            name: quoted("name =>"),
            version: quoted("version =>"),
            sources,
        });
    }
    out
}

fn temp_store() -> (tempfile::TempDir, ChunkStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ChunkStore::open(dir.path().join("store")).expect("open store");
    (dir, store)
}

fn is_cell_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[test]
fn the_seed_recipes_name_one_resolvable_cell_each() {
    let parsed = recipes(BOOTSTRAP);
    let names: Vec<&str> = parsed.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["bootstrap_seed", "musl", "busybox", "hello"],
        "the §4.2 seed chain, in dependency order"
    );
    let versions: Vec<&str> = parsed.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(versions, ["0.1.0", "1.2.5", "1.36.1", "2.12"]);

    // Every recipe has a known payload, so the ingestion table cannot drift
    // away from the module.
    for r in &parsed {
        assert!(
            PAYLOADS.iter().any(|(recipe, ..)| *recipe == r.name),
            "{} has no known payload in the §3 ingestion table",
            r.name
        );
    }

    let mut seen = std::collections::HashSet::new();
    for r in &parsed {
        assert_eq!(
            r.sources.len(),
            1,
            "{} names {} sources; the pinned payload is one tarball per recipe",
            r.name,
            r.sources.len()
        );
        let id = &r.sources[0];
        assert!(
            is_cell_id(id),
            "{}'s source is not a 32-byte lowercase hex identity: {id}",
            r.name
        );
        assert!(seen.insert(id.clone()), "{} reuses a cell id", r.name);
        unhex(id).expect("a pinned digest must decode");
    }

    // The identity is name + version + content: the same payload under the
    // same name and version reaches the same id, and *any* of the three
    // changing moves it. That is what makes the pinned digests reproducible
    // from the `put` commands in the plan, and it is checked with a
    // generated payload here so it needs no store contents.
    let (_dir, store) = temp_store();
    for r in &parsed {
        let payload = format!("round-trip payload for {} {}\n", r.name, r.version);
        let m = store
            .put_buffer(&r.name, &r.version, payload.as_bytes(), None)
            .expect("put");
        let id = m.identity_digest();
        let resolved = store.get_manifest(&id).expect("identity resolves");
        assert_eq!(resolved.name, r.name);
        assert_eq!(resolved.version, r.version);
        assert_eq!(
            store.read_assembled(&resolved).expect("hydrate"),
            payload.as_bytes(),
            "{} must round-trip byte-exact",
            r.name
        );
        assert_eq!(
            store
                .put_buffer(&r.name, &r.version, payload.as_bytes(), None)
                .expect("re-put")
                .identity_digest(),
            id,
            "re-ingesting the same name+version+content must reproduce the id"
        );
        assert_ne!(
            store
                .put_buffer(&r.name, "0.0.1-different", payload.as_bytes(), None)
                .expect("put other version")
                .identity_digest(),
            id,
            "the version is part of the identity"
        );
        let mut changed = payload.clone().into_bytes();
        changed[0] ^= 1;
        assert_ne!(
            store
                .put_buffer(&r.name, &r.version, &changed, None)
                .expect("put changed")
                .identity_digest(),
            id,
            "the content is part of the identity"
        );
        // A pinned digest names one payload, so a store holding a different
        // payload with the same name and version cannot satisfy it.
        let pinned = unhex(&r.sources[0]).expect("pinned digest");
        assert!(
            store.get_manifest(&pinned).is_err(),
            "{}'s pinned cell must not be satisfied by a different payload",
            r.name
        );
    }
}

/// The real payloads, byte-exact. `PLAN_avm_pkg.md` §3 has the ingestion
/// commands; the tarballs and the store are git-ignored, so this is ignored
/// by default. Selecting it (`--ignored`, or `--exact` by name) *asks* for the
/// real-payload check, so absent payloads fail loudly rather than returning: a
/// returning test is reported `ok` by the harness, which would be exactly the
/// "cannot run recorded as passing" this repo's honesty rule forbids.
#[test]
#[ignore = "needs target/pkg-store + target/pkg-sources (PLAN_avm_pkg.md §3)"]
fn real_bootstrap_payloads_resolve_and_round_trip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let store_dir = root.join("target/pkg-store");
    let sources = root.join("target/pkg-sources");
    if !store_dir.is_dir() || !sources.is_dir() {
        panic!(
            "NOT COVERED (never a pass): {} and {} are missing — ingest the \
             tarballs as PLAN_avm_pkg.md §3 shows, or do not select this test",
            store_dir.display(),
            sources.display()
        );
    }
    let store = ChunkStore::open(&store_dir).expect("open the populated store");
    for r in recipes(BOOTSTRAP) {
        let (_, cell_name, cell_version, tarball) = *PAYLOADS
            .iter()
            .find(|(recipe, ..)| *recipe == r.name)
            .unwrap_or_else(|| panic!("no payload known for recipe {}", r.name));
        let id = unhex(&r.sources[0]).expect("pinned digest");
        let m = store
            .get_manifest(&id)
            .unwrap_or_else(|e| panic!("{}: pinned source does not resolve: {e}", r.name));
        assert_eq!(m.name, cell_name, "the source is the {cell_name} cell");
        assert_eq!(m.version, cell_version, "the source cell's version");
        let expected = std::fs::read(sources.join(tarball)).expect("tarball");
        assert_eq!(
            store.read_assembled(&m).expect("hydrate"),
            expected,
            "{} must hydrate to the ingested tarball, byte for byte",
            r.name
        );
    }
}
