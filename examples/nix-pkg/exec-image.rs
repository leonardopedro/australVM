//! exec-image — the caller-supplied executor for `avm-pkg load --exec`.
//!
//! The package engine deliberately never jumps into a cell's machine code
//! (`avm-exec` is `unsafe`-free). `load --exec` stages the *relocated* image
//! as `cell.json` and spawns this program in the runner's sandbox; mapping
//! and calling the image is therefore the caller's job. This file is that
//! caller for the `examples/nix-pkg` integration test.
//!
//! It maps the image at the base `avm-pkg load --base` was given, maps every
//! provider image at the base the loader recorded for it (so the owner's
//! call sites, already patched with those addresses, land in real provider
//! code), resolves the arithmetic helpers to fixed-address stubs (the loader
//! was told the same addresses with `--resolve`), calls `main` and prints the
//! returned word. The CPS engine passes every value in one 64-bit register,
//! so a Float64 result is its IEEE-754 bit pattern in `rax`; the line carries
//! both readings and the caller checks the one its module returns:
//!
//! ```text
//! rax=<i64> f64_bits=<float from those bits>
//! ```
//!
//! x86_64-linux only. The stubs do not trap on overflow — these cells
//! compute small constants; `cps2cell`'s own trapping path is unchanged and
//! tested in the bridge. Compile with plain `rustc` (no cargo, no deps).

use std::ffi::{c_void, OsString};
use std::io::Write;

const CELL_BASE: u64 = 0x1000_0000;
const STUB_BASE: u64 = 0x2000_0000;
const PAGE: usize = 4096;

// Helpers the CPS lowering calls. `cell_builder`'s cells import them by
// these names; the loader patches call sites with whatever address the
// caller passes to `--resolve`, so run.sh passes the addresses below.
const TRAPPING_MUL: u64 = STUB_BASE + 16;
// 32-byte stride: each stub is 20 bytes, so 16 would overlap its neighbour.
const F_ADD: u64 = STUB_BASE + 32;
const F_SUB: u64 = STUB_BASE + 64;
const F_MUL: u64 = STUB_BASE + 96;
const F_DIV: u64 = STUB_BASE + 128;

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod imp {
    use super::*;

    extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
    }

    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 0x02;
    const MAP_ANONYMOUS: i32 = 0x20;
    const MAP_FIXED: i32 = 0x10;

    /// Map an anonymous region at an exact address with the given protection.
    fn map_at(addr: u64, len: usize, prot: i32) -> Result<*mut u8, String> {
        let p = unsafe {
            mmap(
                addr as *mut c_void,
                len,
                prot,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                -1,
                0,
            )
        };
        if p as isize == -1 {
            return Err(format!("mmap at {addr:#x} ({len} bytes) failed"));
        }
        Ok(p as *mut u8)
    }

    /// Integer, then Float64, helper stubs.
    ///
    /// The integer ones compute in rax; the `au_f*` ones take two IEEE bit
    /// patterns in rdi/rsi and return one in rax, which is the CPS one-word
    /// ABI (`Compiler_cps.float_arith_helper`). Overflow trapping is
    /// deliberately absent — these are small-constant test cells (see the
    /// header).
    fn install_stubs() -> Result<(), String> {
        let page = map_at(STUB_BASE, PAGE, PROT_READ | PROT_WRITE)?;
        // trappingAdd: rax = rdi + rsi; ret
        let add: [u8; 7] = [
            0x48, 0x89, 0xf8, // mov rax, rdi
            0x48, 0x01, 0xf0, // add rax, rsi
            0xc3, // ret
        ];
        // trappingMultiply: rax = rdi * rsi; ret
        let mul: [u8; 8] = [
            0x48, 0x89, 0xf8, // mov rax, rdi
            0x48, 0x0f, 0xaf, 0xc6, // imul rax, rsi
            0xc3, // ret
        ];
        // The `au_f*` helpers take the two Float64 bit patterns in rdi/rsi
        // and return the result's bit pattern in rax (the CPS one-word ABI):
        // movq xmm0, rdi; movq xmm1, rsi; <op>sd xmm0, xmm1; movq rax, xmm0.
        // The last byte of each op is the SSE opcode: add 0x58, sub 0x5c,
        // mul 0x59, div 0x5e.
        let float_stub = |sse: u8| -> [u8; 20] {
            [
                0x66, 0x48, 0x0f, 0x6e, 0xc7, // movq xmm0, rdi
                0x66, 0x48, 0x0f, 0x6e, 0xce, // movq xmm1, rsi
                0xf2, 0x0f, sse, 0xc1, // <op>sd xmm0, xmm1
                0x66, 0x48, 0x0f, 0x7e, 0xc0, // movq rax, xmm0
                0xc3, // ret
            ]
        };
        let fadd = float_stub(0x58);
        let fsub = float_stub(0x5c);
        let fmul = float_stub(0x59);
        let fdiv = float_stub(0x5e);
        unsafe {
            std::ptr::copy_nonoverlapping(add.as_ptr(), page, add.len());
            let at = |addr: u64| page.add((addr - STUB_BASE) as usize);
            std::ptr::copy_nonoverlapping(mul.as_ptr(), at(TRAPPING_MUL), mul.len());
            std::ptr::copy_nonoverlapping(fadd.as_ptr(), at(F_ADD), fadd.len());
            std::ptr::copy_nonoverlapping(fsub.as_ptr(), at(F_SUB), fsub.len());
            std::ptr::copy_nonoverlapping(fmul.as_ptr(), at(F_MUL), fmul.len());
            std::ptr::copy_nonoverlapping(fdiv.as_ptr(), at(F_DIV), fdiv.len());
        }
        if unsafe { mprotect(page as *mut c_void, PAGE, PROT_READ | PROT_EXEC) } != 0 {
            return Err("mprotect of the stub page failed".into());
        }
        Ok(())
    }

    fn hex_bytes(s: &str) -> Result<Vec<u8>, String> {
        if s.len() % 2 != 0 {
            return Err("odd-length hex text".into());
        }
        let mut out = Vec::with_capacity(s.len() / 2);
        let b = s.as_bytes();
        let nib = |c: u8| -> Result<u8, String> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(format!("bad hex digit {:?}", c as char)),
            }
        };
        for pair in b.chunks(2) {
            out.push((nib(pair[0])? << 4) | nib(pair[1])?);
        }
        Ok(out)
    }

    /// The `providers` array: each entry's base and relocated text. The
    /// loader emits only `{"base": "0x…", "text": "…"}` objects there, so
    /// scanning pairs after the `"providers"` key is exact; any other shape
    /// (missing base, missing text, odd hex) fails loudly.
    fn providers_from(doc: &str) -> Result<Vec<(u64, Vec<u8>)>, String> {
        let key = "\"providers\":";
        let Some(start) = doc.find(key) else {
            return Ok(Vec::new());
        };
        let mut rest = &doc[start + key.len()..];
        let mut out = Vec::new();
        while let Some(bi) = rest.find("\"base\":") {
            let after = rest[bi + 7..].trim_start();
            let after = after
                .strip_prefix('"')
                .ok_or_else(|| "a provider `base` is not a string".to_string())?;
            let end = after
                .find('"')
                .ok_or_else(|| "a provider `base` is unterminated".to_string())?;
            let base = hex_u64(&after[..end])?;
            let after_base = &after[end..];
            let ti = after_base
                .find("\"text\":")
                .ok_or_else(|| "a provider entry has no `text`".to_string())?;
            let after_text = after_base[ti + 7..].trim_start();
            let after_text = after_text
                .strip_prefix('"')
                .ok_or_else(|| "a provider `text` is not a string".to_string())?;
            let tend = after_text
                .find('"')
                .ok_or_else(|| "a provider `text` is unterminated".to_string())?;
            out.push((base, hex_bytes(&after_text[..tend])?));
            rest = &after_text[tend..];
        }
        Ok(out)
    }

    /// The fields this executor needs, read from the staged image
    /// document. A full JSON parser would be a dependency; the loader's
    /// shape is fixed and these scans fail loudly when it changes.
    fn field_from<'a>(doc: &'a str, key: &str) -> Result<&'a str, String> {
        let pat = format!("\"{key}\":");
        let start = doc
            .find(&pat)
            .ok_or_else(|| format!("cell.json has no `{key}`"))?;
        let rest = &doc[start + pat.len()..];
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix('"')
            .ok_or_else(|| format!("`{key}` is not a string"))?;
        let end = rest
            .find('"')
            .ok_or_else(|| format!("`{key}` is unterminated"))?;
        Ok(&rest[..end])
    }

    fn hex_u64(v: &str) -> Result<u64, String> {
        u64::from_str_radix(v.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad hex address {v:?}: {e}"))
    }

    pub fn main() -> Result<(), String> {
        let mut image: Option<std::path::PathBuf> = None;
        let mut args = std::env::args_os().skip(1);
        while let Some(a) = args.next() {
            if a == OsString::from("--image") {
                image = Some(
                    args.next()
                        .map(std::path::PathBuf::from)
                        .ok_or_else(|| "--image needs a path".to_string())?,
                );
            }
        }
        let image = image.ok_or_else(|| "usage: exec-image --image cell.json".to_string())?;
        let doc = std::fs::read_to_string(&image)
            .map_err(|e| format!("read {}: {e}", image.display()))?;

        let base = hex_u64(field_from(&doc, "base")?)?;
        if base != CELL_BASE {
            return Err(format!(
                "image base {base:#x} is not the {CELL_BASE:#x} this executor maps; \
                 pass `avm-pkg load --base {CELL_BASE:#x}`"
            ));
        }
        let text = hex_bytes(field_from(&doc, "text")?)?;
        // The symbol table is an object; `main` is the entry point.
        let sym_start = doc
            .find("\"symbols\":")
            .ok_or_else(|| "cell.json has no `symbols`".to_string())?;
        let entry = {
            let rest = &doc[sym_start..];
            let pat = "\"main\":";
            let i = rest
                .find(pat)
                .ok_or_else(|| "cell.json's symbol table has no `main`".to_string())?;
            let after = rest[i + pat.len()..].trim_start();
            let after = after
                .strip_prefix('"')
                .ok_or_else(|| "main's address is not a string".to_string())?;
            let end = after.find('"').ok_or_else(|| "main's address is unterminated".to_string())?;
            hex_u64(&after[..end])?
        };

        install_stubs()?;
        let len = text.len().div_ceil(PAGE) * PAGE;

        // Providers are mapped exactly where the loader linked them: the
        // owner's patched call sites carry these absolute addresses. Refuse
        // an overlapping or unaligned layout rather than silently mapping
        // over the owner or the stub page.
        let providers = providers_from(&doc)?;
        let mut spans: Vec<(u64, u64, &str)> = vec![
            (base, base + len as u64, "owner"),
            (STUB_BASE, STUB_BASE + PAGE as u64, "stub page"),
        ];
        for (pbase, ptext) in &providers {
            let plen = ptext.len().div_ceil(PAGE) * PAGE;
            if pbase % PAGE as u64 != 0 {
                return Err(format!("provider base {pbase:#x} is not page-aligned"));
            }
            let pend = pbase
                .checked_add(plen as u64)
                .ok_or_else(|| format!("provider at {pbase:#x} overflows"))?;
            for (s, e, what) in &spans {
                if *pbase < *e && *s < pend {
                    return Err(format!(
                        "provider [{pbase:#x}, {pend:#x}) overlaps {what} [{s:#x}, {e:#x})"
                    ));
                }
            }
            spans.push((*pbase, pend, "provider"));
        }
        for (pbase, ptext) in &providers {
            let plen = ptext.len().div_ceil(PAGE) * PAGE;
            let pcode = map_at(*pbase, plen, PROT_READ | PROT_WRITE)?;
            unsafe {
                std::ptr::copy_nonoverlapping(ptext.as_ptr(), pcode, ptext.len());
            }
            if unsafe { mprotect(pcode as *mut c_void, plen, PROT_READ | PROT_EXEC) } != 0 {
                return Err(format!("mprotect of provider {pbase:#x} failed"));
            }
        }

        let code = map_at(base, len, PROT_READ | PROT_WRITE)?;
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), code, text.len());
        }
        if unsafe { mprotect(code as *mut c_void, len, PROT_READ | PROT_EXEC) } != 0 {
            return Err("mprotect of the code page failed".into());
        }
        if entry < base || entry >= base + len as u64 {
            return Err(format!(
                "main at {entry:#x} is outside the mapped image [{base:#x}, {:#x})",
                base + len as u64
            ));
        }

        // One call: the engine returns every value in rax (Int64, Bool as
        // 0/1, and Float64 as its IEEE bit pattern).
        let as_int: extern "C" fn() -> i64 = unsafe { std::mem::transmute(entry as usize) };
        let rax = as_int();

        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "rax={rax} f64_bits={:?}",
            f64::from_bits(rax as u64)
        )
        .map_err(|e| format!("stdout: {e}"))?;
        Ok(())
    }
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
mod imp {
    pub fn main() -> Result<(), String> {
        Err("exec-image is x86_64-linux only".into())
    }
}

fn main() {
    if let Err(e) = imp::main() {
        eprintln!("exec-image: {e}");
        std::process::exit(2);
    }
}
