//! Hot-store dynamic linking (spec task 3.3): *symbol resolution that
//! requests missing dependencies from `EphemeralHotStore::acquire_cell()`,
//! demonstrating a cell resolving an imported function directly from
//! another mapped cell's address space.*
//!
//! The linker never writes into a resident mapping (those are read-only
//! by construction — zero-writeback is an invariant). It produces a
//! *patched copy* of the owner cell's text in which every relocation
//! carries the provider's **real mapped address**, which is exactly the
//! resolution the spec asks to demonstrate: the addend arithmetic below
//! uses the provider's `mmap` base, not a symbolic stand-in.
//!
//! Supported relocation kinds (from the cell's K region, Debug-named by
//! the bridge): `Abs8`, `Abs4`, `X86PCRel4`, `X86CallPCRel4`,
//! `X86CallPLTRel4` (treated as a direct rel32 call — correct in a single
//! address space where the provider's VA is known, which is this system's
//! invariant 5 model). GOT/PLT-style indirection and anything unknown are
//! **hard errors**: a cell that needs a relocation table we cannot apply
//! must fail loudly rather than link wrong.

use crate::cell_builder::{AvceReloc, CellError, CellView};
use std::collections::BTreeMap;

/// The owner cell's text after relocation, plus the absolute addresses of
/// every symbol that ended up in the image (chained linking can use these
/// as further providers).
#[derive(Debug)]
pub struct LinkedImage {
    pub text: Vec<u8>,
    /// name → absolute address (owner and providers alike).
    pub symbols: BTreeMap<String, u64>,
}

/// A provider cell: where its mapping actually lives.
pub struct Provider<'a, 'b> {
    pub view: &'b CellView<'a>,
    pub base: u64,
}

fn apply_one(text: &mut [u8], site_base: u64, r: &AvceReloc, target: u64) -> Result<(), CellError> {
    let off = r.offset as usize;
    let write32 = |text: &mut [u8], bytes: [u8; 4]| -> Result<(), CellError> {
        let end = off
            .checked_add(4)
            .ok_or_else(|| CellError::Link("offset overflow".into()))?;
        if end > text.len() {
            return Err(CellError::Link(format!(
                "reloc at {off} runs past the text ({} bytes)",
                text.len()
            )));
        }
        text[off..end].copy_from_slice(&bytes);
        Ok(())
    };
    let write64 = |text: &mut [u8], bytes: [u8; 8]| -> Result<(), CellError> {
        let end = off
            .checked_add(8)
            .ok_or_else(|| CellError::Link("offset overflow".into()))?;
        if end > text.len() {
            return Err(CellError::Link(format!(
                "reloc at {off} runs past the text ({} bytes)",
                text.len()
            )));
        }
        text[off..end].copy_from_slice(&bytes);
        Ok(())
    };

    match r.kind.as_str() {
        "Abs8" => {
            let val = (target as i128 + i128::from(r.addend)) as u64;
            write64(text, val.to_le_bytes())
        }
        "Abs4" => {
            let val = target as i128 + i128::from(r.addend);
            let val = i32::try_from(val)
                .map_err(|_| CellError::Link(format!("Abs4 target {val} out of range")))?;
            write32(text, val.to_le_bytes())
        }
        "X86PCRel4" | "X86CallPCRel4" | "X86CallPLTRel4" => {
            let site = site_base
                .checked_add(r.offset)
                .ok_or_else(|| CellError::Link("site address overflow".into()))?;
            // Cranelift's `MachReloc` addend already carries the -4 for a
            // rel32 field (measured on a `cps2cell` cell: `X86CallPCRel4`,
            // addend -4). So this is the ELF rule `S + A - P`: adding the
            // field size again double-counts it and puts every internal
            // call four bytes past its target.
            let rel = (target as i128 + i128::from(r.addend)) - site as i128;
            let rel = i32::try_from(rel).map_err(|_| {
                CellError::Link(format!(
                    "PC-relative distance {rel} from {site:#x} to {target:#x} out of i32 range \
                     (provider cell more than 2 GiB away)"
                ))
            })?;
            write32(text, rel.to_le_bytes())
        }
        "X86GOTPCRel4" | "X86GOTPCRel8" | "X86CallPLTRel4GOT" => Err(CellError::Link(format!(
            "relocation kind {} needs GOT/PLT indirection the cell format does not carry",
            r.kind
        ))),
        other => Err(CellError::Link(format!(
            "unknown relocation kind {other:?}"
        ))),
    }
}

/// Refuse reloc kinds this linker cannot apply — checked *before* symbol
/// resolution, so a format-level problem reports as itself even when the
/// symbol table is also incomplete.
fn check_kind(kind: &str) -> Result<(), CellError> {
    match kind {
        "Abs8" | "Abs4" | "X86PCRel4" | "X86CallPCRel4" | "X86CallPLTRel4" => Ok(()),
        "X86GOTPCRel4" | "X86GOTPCRel8" | "X86CallPLTRel4GOT" => Err(CellError::Link(format!(
            "relocation kind {kind} needs GOT/PLT indirection the cell format does not carry"
        ))),
        other => Err(CellError::Link(format!(
            "unknown relocation kind {other:?}"
        ))),
    }
}

/// A multi-cell image: the owner's text plus one patched text per provider,
/// all relocated against the same symbol table.
pub struct LinkedCells {
    pub owner: LinkedImage,
    /// Provider texts in the order the providers were given, each as if
    /// mapped at its own base.
    pub providers: Vec<LinkedImage>,
}

/// Relocate `owner`'s text as if mapped at `owner_base`, resolving
/// undefined symbols against `providers` (already mapped at their real
/// addresses). Returns the patched text and all known symbol addresses.
///
/// Local definitions shadow providers; a symbol that is neither local nor
/// provided is a hard error (never a silent zero).
pub fn link<'a, 'v>(
    owner: &CellView<'a>,
    owner_base: u64,
    providers: &[Provider<'a, 'v>],
) -> Result<LinkedImage, CellError> {
    link_with(owner, owner_base, providers, &BTreeMap::new())
}

/// [`link`] plus a runtime symbol table: `externs` maps a name to the
/// absolute address a runtime registered it at (the JIT's `uk_*`/`uz_*`
/// set, for instance). Resolution order is owner definitions, then
/// providers, then externs — a local definition always wins, and an
/// unresolved name is still a hard error rather than a silent zero.
pub fn link_with<'a, 'v>(
    owner: &CellView<'a>,
    owner_base: u64,
    providers: &[Provider<'a, 'v>],
    externs: &BTreeMap<String, u64>,
) -> Result<LinkedImage, CellError> {
    let cells = link_cells(owner, owner_base, providers, externs)?;
    Ok(cells.owner)
}

/// Refuse layout and relocation-kind problems in `view` before any symbol
/// is resolved, so a format-level problem reports as itself even when the
/// symbol table is also incomplete.
fn validate(view: &CellView<'_>) -> Result<(), CellError> {
    if view.code_len() > view.text.len() {
        return Err(CellError::Link("symdefs run past the text".into()));
    }
    for r in &view.relocs {
        check_kind(&r.kind)?;
    }
    Ok(())
}

/// The shared symbol table: owner definitions first (duplicates there are an
/// error), then each provider's definitions, then the runtime table. Provider
/// definitions never overwrite an earlier one — a provider cannot shadow the
/// owner, and the first provider wins between providers, matching [`link`]'s
/// documented precedence.
fn symbol_table<'a, 'v>(
    owner: &CellView<'a>,
    owner_base: u64,
    providers: &[Provider<'a, 'v>],
    externs: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, u64>, CellError> {
    let mut symbols: BTreeMap<String, u64> = BTreeMap::new();
    for s in &owner.symdefs {
        if symbols
            .insert(s.name.clone(), owner_base + s.offset)
            .is_some()
        {
            return Err(CellError::Link(format!("duplicate symbol {}", s.name)));
        }
    }
    for p in providers {
        for s in &p.view.symdefs {
            if !symbols.contains_key(&s.name) {
                symbols.insert(s.name.clone(), p.base + s.offset);
            }
        }
    }
    for (name, addr) in externs {
        if !symbols.contains_key(name) {
            symbols.insert(name.clone(), *addr);
        }
    }
    Ok(symbols)
}

/// Copy one cell's code and apply its relocations against `symbols`.
fn patch_text(
    view: &CellView<'_>,
    base: u64,
    symbols: &BTreeMap<String, u64>,
    provider_symbols: usize,
    externs_len: usize,
) -> Result<Vec<u8>, CellError> {
    // Working copy: real code only (page padding excluded), never the
    // resident mapping itself.
    let mut text = view.text[..view.code_len()].to_vec();
    for r in &view.relocs {
        let target = *symbols.get(&r.symbol).ok_or_else(|| {
            CellError::Link(format!(
                "unresolved symbol {:?} (owner defines {} symbol(s), providers supply {}, \
                 the runtime table has {})",
                r.symbol,
                view.symdefs.len(),
                provider_symbols,
                externs_len,
            ))
        })?;
        apply_one(&mut text, base, r, target)?;
    }
    Ok(text)
}

/// Relocate a whole image: the owner *and* every provider cell, each at its
/// own base, against one shared symbol table.
///
/// A provider is not just a symbol donor here — its own relocations are
/// applied too, so a provider that calls another cell's function or the
/// runtime runs from its patched copy. This is the difference from
/// [`link_with`], which patches only the owner and leaves provider text
/// untouched.
pub fn link_cells<'a, 'v>(
    owner: &CellView<'a>,
    owner_base: u64,
    providers: &[Provider<'a, 'v>],
    externs: &BTreeMap<String, u64>,
) -> Result<LinkedCells, CellError> {
    validate(owner)?;
    for p in providers {
        validate(p.view)?;
    }

    let symbols = symbol_table(owner, owner_base, providers, externs)?;
    let provider_symbols = providers
        .iter()
        .map(|p| p.view.symdefs.len())
        .sum::<usize>();
    let owner_image = LinkedImage {
        text: patch_text(owner, owner_base, &symbols, provider_symbols, externs.len())?,
        symbols: symbols.clone(),
    };
    let mut provider_images = Vec::with_capacity(providers.len());
    for p in providers {
        provider_images.push(LinkedImage {
            text: patch_text(p.view, p.base, &symbols, provider_symbols, externs.len())?,
            symbols: symbols.clone(),
        });
    }

    // Every import must have resolved too — an import with no reloc is
    // fine (nothing to patch), but a defined-but-unpatched import name in
    // the symbol table is expected: runtime primitives resolve at load.
    Ok(LinkedCells {
        owner: owner_image,
        providers: provider_images,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_builder::{build_cell, AvceDoc, AvceFn, BuildCell};

    /// Byte-level view constructor: a cell with *no* signature check —
    /// these tests care about layout arithmetic, and `build_cell` is
    /// covered in cell_builder's own tests. (Signed properly anyway.)
    fn make_cell(avce: &AvceDoc) -> Vec<u8> {
        build_cell(&BuildCell {
            drv_hash: [0x42u8; 32],
            capability_table: &serde_json::json!([]),
            avce,
            signing_seed: &[3u8; 32],
        })
        .unwrap()
    }

    /// Task 3.3's demonstration: an owner cell whose call site resolves
    /// against a provider **mapped by the hot store**, at that mapping's
    /// real address — asserted byte-for-byte.
    #[test]
    fn resolves_symbol_from_mapped_provider_cell() {
        // Provider: `mov eax, 7; ret` under the name give_seven.
        let provider_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "give_seven".into(),
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![],
            }],
            imports: vec![],
        };
        // Owner: `call give_seven; ret` — rel32 slot at offset 1.
        let owner_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "caller".into(),
                body: vec![0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![crate::cell_builder::AvceReloc {
                    offset: 1,
                    // Cranelift's convention for a rel32 call site; the
                    // linker must not add the field size a second time.
                    addend: -4,
                    kind: "X86CallPCRel4".into(),
                    symbol: "give_seven".into(),
                }],
            }],
            imports: vec!["give_seven".into()],
        };

        // Ingest both into a real cold store and hydrate them through the
        // ephemeral hot store — real mmaps, real addresses.
        let dir = tempfile::tempdir().unwrap();
        let cold =
            std::sync::Arc::new(avm_cas::ChunkStore::open(dir.path().join("store")).unwrap());
        let hot = avm_cas::HotStore::new(
            cold.clone(),
            16 * 1024 * 1024,
            dir.path().join("hot"),
            false,
        )
        .unwrap();

        let provider_cell = make_cell(&provider_avce);
        let owner_cell = make_cell(&owner_avce);
        let pm = cold
            .put_buffer("provider", "1", &provider_cell, None)
            .unwrap();
        let om = cold.put_buffer("owner", "1", &owner_cell, None).unwrap();

        let pmap = hot.acquire(&pm.identity_digest()).unwrap();
        let omap = hot.acquire(&om.identity_digest()).unwrap();

        let pview = CellView::parse(&pmap).unwrap();
        let oview = CellView::parse(&omap).unwrap();

        let provider_base = pmap.as_ref().as_ptr() as u64;
        let owner_base = omap.as_ref().as_ptr() as u64;

        let image = link(
            &oview,
            owner_base,
            &[Provider {
                view: &pview,
                base: provider_base,
            }],
        )
        .unwrap();

        // The rel32 at offset 1 must land EXACTLY on give_seven's first
        // instruction inside the provider's live mapping.
        let got = i32::from_le_bytes(image.text[1..5].try_into().unwrap());
        let want = (provider_base as i128 - (owner_base as i128 + 1 + 4)) as i32;
        assert_eq!(
            got, want,
            "call site must resolve to the provider mapping (base {provider_base:#x})"
        );
        assert_eq!(image.text[0], 0xE8, "opcode untouched");
        assert_eq!(image.text[5], 0xC3, "tail ret untouched");
        assert_eq!(
            *image.symbols.get("give_seven").unwrap(),
            provider_base,
            "symbol table records the provider's real address"
        );

        // The resident mapping itself was never dirtied: its *text*
        // region still holds the unrelocated zeros (the patch went to the
        // working copy only).
        assert_eq!(
            oview.text[1..5],
            [0x00, 0x00, 0x00, 0x00],
            "owner mapping pristine"
        );
        drop(pmap);
        drop(omap);
    }

    #[test]
    fn unresolved_symbol_is_a_hard_error() {
        let owner_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "caller".into(),
                body: vec![0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![crate::cell_builder::AvceReloc {
                    offset: 1,
                    addend: 0,
                    kind: "X86CallPCRel4".into(),
                    symbol: "nowhere".into(),
                }],
            }],
            imports: vec![],
        };
        let cell = make_cell(&owner_avce);
        let view = CellView::parse(&cell).unwrap();
        let err = link(&view, 0x1000, &[]).unwrap_err();
        assert!(
            matches!(err, CellError::Link(_)),
            "unresolved must fail: {err}"
        );
        assert!(err.to_string().contains("nowhere"));
    }

    #[test]
    fn unsupported_reloc_kind_is_a_hard_error() {
        let owner_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "caller".into(),
                body: vec![0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![crate::cell_builder::AvceReloc {
                    offset: 1,
                    addend: 0,
                    kind: "X86GOTPCRel4".into(),
                    symbol: "give_seven".into(),
                }],
            }],
            imports: vec![],
        };
        let cell = make_cell(&owner_avce);
        let view = CellView::parse(&cell).unwrap();
        let err = link(&view, 0x1000, &[]).unwrap_err();
        assert!(err.to_string().contains("GOT"), "GOT refused loudly: {err}");
    }

    /// A provider's *own* relocations must be applied too, against the same
    /// table: here provider -> owner and owner -> provider, so neither side
    /// can be right unless the table is shared and both texts patched.
    #[test]
    fn multi_cell_link_relocates_providers_against_the_shared_table() {
        // Owner: helper (6 bytes) + start, whose (function-relative) call
        // at offset 1 targets the provider's call_helper.
        let owner_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![
                AvceFn {
                    name: "helper".into(),
                    body: vec![0xB8, 0x09, 0x00, 0x00, 0x00, 0xC3],
                    relocs: vec![],
                },
                AvceFn {
                    name: "start".into(),
                    body: vec![0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
                    relocs: vec![crate::cell_builder::AvceReloc {
                        offset: 1,
                        addend: -4,
                        kind: "X86CallPCRel4".into(),
                        symbol: "call_helper".into(),
                    }],
                },
            ],
            imports: vec!["call_helper".into()],
        };
        // Provider: call_helper, whose call at its own offset 1 targets the
        // owner's helper. Its text must be patched for that to work.
        let provider_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "call_helper".into(),
                body: vec![0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![crate::cell_builder::AvceReloc {
                    offset: 1,
                    addend: -4,
                    kind: "X86CallPCRel4".into(),
                    symbol: "helper".into(),
                }],
            }],
            imports: vec!["helper".into()],
        };
        let owner = make_cell(&owner_avce);
        let provider = make_cell(&provider_avce);
        let ov = CellView::parse(&owner).unwrap();
        let pv = CellView::parse(&provider).unwrap();
        let owner_base = 0x1000_0000u64;
        let provider_base = 0x1000_1000u64;

        let cells = link_cells(
            &ov,
            owner_base,
            &[Provider {
                view: &pv,
                base: provider_base,
            }],
            &BTreeMap::new(),
        )
        .unwrap();

        // Owner's call lands on the provider's call_helper (offset 0).
        let owner_rel = i32::from_le_bytes(cells.owner.text[7..11].try_into().unwrap());
        assert_eq!(
            owner_rel as i128,
            provider_base as i128 - 4 - (owner_base as i128 + 7)
        );
        // Provider's call lands on the owner's helper (offset 0) — this is
        // the assertion `link_with` cannot make, because it does not patch
        // provider text at all.
        assert_eq!(cells.providers.len(), 1);
        let provider_rel = i32::from_le_bytes(cells.providers[0].text[1..5].try_into().unwrap());
        assert_eq!(
            provider_rel as i128,
            owner_base as i128 - 4 - (provider_base as i128 + 1)
        );
        assert_eq!(
            *cells.providers[0].symbols.get("helper").unwrap(),
            owner_base,
            "the table is shared, so a provider sees the owner's symbols"
        );
    }

    #[test]
    fn abs8_reloc_patches_full_address() {
        let owner_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "grabber".into(),
                body: vec![0x48, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0xC3], // movabs rax, imm64; ret
                relocs: vec![crate::cell_builder::AvceReloc {
                    offset: 2,
                    addend: 0x10,
                    kind: "Abs8".into(),
                    symbol: "data_blob".into(),
                }],
            }],
            imports: vec![],
        };
        let provider_avce = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "data_blob".into(),
                body: vec![0x00, 0x01, 0x02, 0x03],
                relocs: vec![],
            }],
            imports: vec![],
        };
        let owner_cell = make_cell(&owner_avce);
        let provider_cell = make_cell(&provider_avce);
        let ov = CellView::parse(&owner_cell).unwrap();
        let pv = CellView::parse(&provider_cell).unwrap();
        let provider_base = 0xAAAA_0000u64;
        let image = link(
            &ov,
            0x4000,
            &[Provider {
                view: &pv,
                base: provider_base,
            }],
        )
        .unwrap();
        let got = u64::from_le_bytes(image.text[2..10].try_into().unwrap());
        assert_eq!(got, provider_base + 0x10, "Abs8 = provider base + addend");
    }
}
