//! Cell capture tests (spec §3.4, Task 3.2): same CPS compiler as
//! `cps_tests.rs`, but published through `CaptureModule` into the `AVCE`
//! container that `avm-exec`'s cell builder consumes.

use austral_cranelift_bridge::capture::CaptureModule;
use austral_cranelift_bridge::cps;

// ── CPS v1 fixtures (wire layout mirrors tests/cps_tests.rs) ────────

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn push_fn(buf: &mut Vec<u8>, name: &str, body: &[u8]) {
    write_str(buf, name);
    write_u32(buf, 0); // param_count
    buf.push(0); // ret_type
    write_u32(buf, body.len() as u32);
    buf.extend_from_slice(body);
}

/// Two functions: `leaf` returns 42; `caller` tail-calls `leaf`.
fn cps_two_funcs() -> Vec<u8> {
    let mut leaf = Vec::new();
    leaf.push(0x07); // return context
    leaf.push(0x01); // iconst
    write_i64(&mut leaf, 42);

    let mut caller = Vec::new();
    caller.push(0x07); // return context
    caller.push(0x04); // App (call)
    write_str(&mut caller, "leaf");
    write_u32(&mut caller, 0); // arg_count

    let mut buf = Vec::new();
    write_u32(&mut buf, 0x4350_5331); // magic v1
    write_u32(&mut buf, 2); // func_count
    push_fn(&mut buf, "leaf", &leaf);
    push_fn(&mut buf, "caller", &caller);
    buf
}

/// Single self-contained function `run` returning 7.
fn cps_single() -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x01);
    write_i64(&mut body, 7);
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x4350_5331);
    write_u32(&mut buf, 1);
    push_fn(&mut buf, "run", &body);
    buf
}

// ── AVCE mini-reader (mirror of avm-exec's parser, by format doc) ───

struct R<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> R<'a> {
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes(self.b[self.i..self.i + 2].try_into().unwrap());
        self.i += 2;
        v
    }
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.i..self.i + 4].try_into().unwrap());
        self.i += 4;
        v
    }
    fn u64(&mut self) -> u64 {
        let v = u64::from_le_bytes(self.b[self.i..self.i + 8].try_into().unwrap());
        self.i += 8;
        v
    }
    fn i64v(&mut self) -> i64 {
        let v = i64::from_le_bytes(self.b[self.i..self.i + 8].try_into().unwrap());
        self.i += 8;
        v
    }
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let v = &self.b[self.i..self.i + n];
        self.i += n;
        v
    }
    fn str(&mut self) -> String {
        let n = self.u32() as usize;
        String::from_utf8(self.bytes(n).to_vec()).unwrap()
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn capture_yields_bodies_and_named_relocs() {
    let mut cap = CaptureModule::new().unwrap();
    let module = cps::compile_cps_to_clif(&mut cap, &cps_two_funcs()).unwrap();
    assert!(module.name_map.contains_key("leaf"), "leaf declared");
    assert!(module.name_map.contains_key("caller"), "caller declared");

    let defined = cap.defined();
    let names: Vec<&str> = defined.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["leaf", "caller"],
        "deterministic FuncId (declaration) order"
    );

    let mut declared: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    declared.extend(cap.imports());

    for (name, body) in &defined {
        assert!(!body.bytes.is_empty(), "function {name} has empty text");
        for r in &body.relocs {
            assert!(
                declared.iter().any(|d| d == &r.symbol),
                "reloc in {name} names unknown symbol {:?} (declared: {declared:?})",
                r.symbol
            );
            assert!(!r.kind.is_empty(), "reloc kind must be recorded");
        }
    }

    // The intra-cell tail call must be a named relocation against `leaf`:
    // this is the table task 3.3's linker resolves between mapped cells.
    let (_, caller) = defined.iter().find(|(n, _)| n == "caller").unwrap();
    let rel = caller
        .relocs
        .iter()
        .find(|r| r.symbol == "leaf")
        .unwrap_or_else(|| {
            panic!(
                "caller must relocate against leaf, got {:#?}",
                caller.relocs
            )
        });
    assert!(
        rel.kind.contains("Call") || rel.kind.contains("PCRel") || rel.kind.contains("Abs"),
        "call relocation kind should be a call/pc-rel/abs form, got {}",
        rel.kind
    );
}

#[test]
fn avce_container_parses_fully_and_deterministically() {
    let mut cap = CaptureModule::new().unwrap();
    cps::compile_cps_to_clif(&mut cap, &cps_single()).unwrap();
    let a = cap.encode_avce();
    let b = cap.encode_avce();
    assert_eq!(a, b, "encoding must be byte-stable");

    let mut r = R { b: &a, i: 0 };
    assert_eq!(r.u32(), 0x4156_4345, "magic 'AVCE'");
    assert_eq!(r.u16(), 1, "version");
    let arch = r.str();
    assert!(!arch.is_empty(), "arch recorded: {arch}");
    assert_eq!(r.u32(), 1, "one defined function");
    let name = r.str();
    assert_eq!(name, "run");
    let body_len = r.u32() as usize;
    assert!(body_len > 0, "text section non-empty");
    let _body = r.bytes(body_len);
    let reloc_count = r.u32();
    for _ in 0..reloc_count {
        let _off = r.u64();
        let _addend = r.i64v();
        let _kind = r.str();
        let _sym = r.str();
    }
    let import_count = r.u32();
    for _ in 0..import_count {
        let _imp = r.str();
    }
    assert_eq!(r.i, a.len(), "parser must consume the container exactly");
}

#[test]
fn imports_from_extern_calls_are_listed_not_defined() {
    // A body tail-calling an external 0-arg primitive (the `au_*` /
    // runtime shape): it must land in `imports`, never in `defined`.
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x04);
    write_str(&mut body, "au_extern_probe");
    write_u32(&mut body, 0);
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x4350_5331);
    write_u32(&mut buf, 1);
    push_fn(&mut buf, "run", &body);

    let mut cap = CaptureModule::new().unwrap();
    cps::compile_cps_to_clif(&mut cap, &buf).unwrap();
    assert_eq!(cap.imports(), vec!["au_extern_probe".to_string()]);
    let defined = cap.defined();
    assert_eq!(defined.len(), 1);
    let (_, def) = &defined[0];
    assert!(def.relocs.iter().any(|r| r.symbol == "au_extern_probe"));
}
