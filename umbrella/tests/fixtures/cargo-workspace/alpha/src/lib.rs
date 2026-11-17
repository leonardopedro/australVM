pub const HALF: i64 = fixture_shared::VALUE * 2;

#[no_mangle]
pub extern "C" fn fixture_alpha_value() -> i64 {
    HALF
}

/// Keeps the proc-macro in this crate's build graph, so the stream really does
/// carry a proc-macro `.so` next to the cdylibs.
pub fn via_macro() -> i64 {
    fixture_macros::fixture_answer!()
}
