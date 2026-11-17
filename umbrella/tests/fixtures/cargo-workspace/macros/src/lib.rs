extern crate proc_macro;

use proc_macro::TokenStream;

/// Enough to make the crate a real proc-macro artifact; the fixture only needs
/// the `.so` it produces to appear in cargo's stream.
#[proc_macro]
pub fn fixture_answer(_input: TokenStream) -> TokenStream {
    "21".parse().expect("literal token stream")
}
