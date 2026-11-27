//! The independent oracle for `cap_fs`: read one file, apply the module's
//! transform and write the result to another. Kept separate from the capability
//! code and the module so the comparison is against a caller that has ambient
//! filesystem access, not against the same abstraction being tested.
//!
//! The transform is the one `CapFs.aum`'s `transform` performs: each complete
//! 8-byte word (little-endian) is increased by one, and a trailing run shorter
//! than a word is copied unchanged. The module's `+` is trapping arithmetic, so
//! the two agree only where no word is all-ones; the example's text records never
//! come close, and the wrapping add below is what the word rule says.
//!
//! Usage: `reference <input> <output>`

use std::process::ExitCode;

fn transform(bytes: &mut [u8]) {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[offset..offset + 8]);
        let value = u64::from_le_bytes(word).wrapping_add(1);
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        offset += 8;
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: reference <input> <output>");
        return ExitCode::from(2);
    }
    let mut bytes = match std::fs::read(&args[1]) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("reference: cannot read {}: {error}", args[1]);
            return ExitCode::FAILURE;
        }
    };
    transform(&mut bytes);
    match std::fs::write(&args[2], &bytes) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reference: cannot write {}: {error}", args[2]);
            ExitCode::FAILURE
        }
    }
}
