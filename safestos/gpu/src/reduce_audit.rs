//! G22b: the structural audit of the staged reduction WGSL.
//!
//! Naga proves the shader parses and type-checks. That is not the same as
//! proving it is *this* collective: a shader can drop a guard, skip a barrier,
//! use the wrong stride, or let every lane write the group's slot and still be
//! valid WGSL. The audit below checks the emitted structure directly and its
//! tests mutate one property at a time, so a passing audit is evidence about
//! the bytes rather than about the renderer's intent.
//!
//! The check is textual on the emitter's fixed output. That is a deliberate
//! trade: the alternative is a full dataflow analysis of WGSL, and the emitter
//! has no user-controlled identifiers in this region (the mapper body is
//! separate and already validated as KernelIR).

use crate::reduce::{IDENTITY, ELEMENTS_PER_LANE, GROUP_ELEMENTS, SCRATCH_BYTES, Strategy, TILE};

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

fn require_count(haystack: &str, needle: &str, expected: usize, what: &str) -> Result<(), String> {
    let found = count(haystack, needle);
    if found != expected {
        return Err(format!(
            "structural audit: {what} appears {found} time(s), expected {expected}"
        ));
    }
    Ok(())
}

/// Position of `needle`, or an audit failure naming it.
fn position(haystack: &str, needle: &str) -> Result<usize, String> {
    haystack
        .find(needle)
        .ok_or_else(|| format!("structural audit: missing {needle:?}"))
}

/// One element per lane (`map-sum-u32`).
pub fn audit(wgsl: &str) -> Result<(), String> {
    let tile = TILE as u32;
    require_count(
        wgsl,
        &format!("@compute @workgroup_size({tile})"),
        2,
        "the two compute entries",
    )?;
    require_count(wgsl, "fn map_sum(", 1, "map_sum entry")?;
    require_count(wgsl, "fn add_partials(", 1, "add_partials entry")?;
    require_count(wgsl, "fn mapper(p0: u32) -> u32", 1, "the mapper function")?;

    // Binding roles: exactly one read-only input and one read_write output.
    require_count(
        wgsl,
        "@group(0) @binding(0) var<storage, read> input0: array<u32>;",
        1,
        "the read-only input binding",
    )?;
    require_count(
        wgsl,
        "@group(0) @binding(1) var<storage, read_write> output0: array<u32>;",
        1,
        "the read_write output binding",
    )?;
    require_count(
        wgsl,
        &format!("var<workgroup> scratch: array<u32, {tile}>;"),
        1,
        "the workgroup scratch",
    )?;
    if (scratch_bytes() as u32) != SCRATCH_BYTES as u32 {
        return Err("structural audit: scratch size disagreement".to_string());
    }

    // Every load guarded, and the tail lane carries the identity.
    require_count(wgsl, "if (i >= arrayLength(&input0))", 2, "the load guard")?;
    require_count(
        wgsl,
        &format!("scratch[lane] = {IDENTITY}u;"),
        2,
        "the identity for a tail lane",
    )?;
    require_count(
        wgsl,
        "scratch[lane] = mapper(input0[i]);",
        1,
        "the mapper applied only to a guarded input",
    )?;
    require_count(
        wgsl,
        "scratch[lane] = input0[i];",
        1,
        "the partial pass reading its guarded input",
    )?;

    // One barrier after the load and one after each stride step, per entry.
    require_count(wgsl, "workgroupBarrier();", 14, "barriers")?;

    // The strides are exactly 32,16,8,4,2,1, in that order.
    let mut previous = 0usize;
    for stride in [32u32, 16, 8, 4, 2, 1] {
        require_count(
            wgsl,
            &format!("if (lane < {stride}u)"),
            2,
            &format!("the stride {stride} guard"),
        )?;
        require_count(
            wgsl,
            &format!("scratch[lane + {stride}u]"),
            2,
            &format!("the stride {stride} partner slot"),
        )?;
        let at = position(wgsl, &format!("if (lane < {stride}u)"))?;
        if at < previous {
            return Err(format!("structural audit: stride {stride} is out of order"));
        }
        previous = at;
    }

    // One write per group, from lane 0, into that group's unique slot. The
    // per-lane refusal is checked first so a rewrite of the only write to
    // `output0[lane]` reports the property it actually broke.
    require_count(wgsl, "output0[lane]", 0, "a per-lane output write")?;
    require_count(wgsl, "if (lane == 0u)", 2, "the single group writer")?;
    require_count(
        wgsl,
        "output0[wid.x] = scratch[0u];",
        2,
        "the group partial write",
    )?;
    Ok(())
}

fn scratch_bytes() -> usize {
    TILE * 4
}

/// Four elements per lane (`map-sum-u32-x4`). The tree reduction is the same;
/// the load phase has four independent guards and accumulates into `acc`, so a
/// lane with a short tail contributes the identity for exactly the missing
/// indices and never maps one.
pub fn audit_x4(wgsl: &str) -> Result<(), String> {
    let tile = TILE as u32;
    let group_elements = GROUP_ELEMENTS as u32;
    require_count(
        wgsl,
        &format!("@compute @workgroup_size({tile})"),
        2,
        "the two compute entries",
    )?;
    require_count(wgsl, "fn map_sum(", 1, "map_sum entry")?;
    require_count(wgsl, "fn add_partials(", 1, "add_partials entry")?;
    require_count(wgsl, "fn mapper(p0: u32) -> u32", 1, "the mapper function")?;

    require_count(
        wgsl,
        "@group(0) @binding(0) var<storage, read> input0: array<u32>;",
        1,
        "the read-only input binding",
    )?;
    require_count(
        wgsl,
        "@group(0) @binding(1) var<storage, read_write> output0: array<u32>;",
        1,
        "the read_write output binding",
    )?;
    require_count(
        wgsl,
        &format!("var<workgroup> scratch: array<u32, {tile}>;"),
        1,
        "the workgroup scratch",
    )?;
    if scratch_bytes() as u32 != SCRATCH_BYTES as u32 {
        return Err("structural audit: scratch size disagreement".to_string());
    }

    // The group covers four tiles at i = group*256 + j*64 + lane.
    require_count(
        wgsl,
        &format!("wid.x * {group_elements}u + lane"),
        2,
        "the four-per-lane base index",
    )?;
    for (j, offset) in [(1u32, TILE), (2, TILE * 2), (3, TILE * 3)] {
        require_count(
            wgsl,
            &format!("let i{j} = i0 + {offset}u;"),
            2,
            &format!("chunk {j}'s offset"),
        )?;
    }
    for j in 0..ELEMENTS_PER_LANE as u32 {
        require_count(
            wgsl,
            &format!("if (i{j} < arrayLength(&input0))"),
            2,
            &format!("the load guard for chunk {j}"),
        )?;
        require_count(
            wgsl,
            &format!("acc = acc + mapper(input0[i{j}]);"),
            1,
            &format!("the mapped load for chunk {j}"),
        )?;
        require_count(
            wgsl,
            &format!("acc = acc + input0[i{j}];"),
            1,
            &format!("the partial load for chunk {j}"),
        )?;
    }
    require_count(
        wgsl,
        &format!("var acc: u32 = {IDENTITY}u;"),
        2,
        "the per-lane accumulator identity",
    )?;
    require_count(wgsl, "scratch[lane] = acc;", 2, "the per-lane slot write")?;

    require_count(wgsl, "workgroupBarrier();", 14, "barriers")?;
    let mut previous = 0usize;
    for stride in [32u32, 16, 8, 4, 2, 1] {
        require_count(
            wgsl,
            &format!("if (lane < {stride}u)"),
            2,
            &format!("the stride {stride} guard"),
        )?;
        require_count(
            wgsl,
            &format!("scratch[lane + {stride}u]"),
            2,
            &format!("the stride {stride} partner slot"),
        )?;
        let at = position(wgsl, &format!("if (lane < {stride}u)"))?;
        if at < previous {
            return Err(format!("structural audit: stride {stride} is out of order"));
        }
        previous = at;
    }
    require_count(wgsl, "output0[lane]", 0, "a per-lane output write")?;
    require_count(wgsl, "if (lane == 0u)", 2, "the single group writer")?;
    require_count(
        wgsl,
        "output0[wid.x] = scratch[0u];",
        2,
        "the group partial write",
    )?;
    Ok(())
}

/// Dispatch on the artifact's operation; an unknown operation is an error, not
/// a default audit.
pub fn audit_for(operation: &str, wgsl: &str) -> Result<(), String> {
    match Strategy::from_operation(operation) {
        Some(Strategy::OnePerLane) => audit(wgsl),
        Some(Strategy::FourPerLane) => audit_x4(wgsl),
        None => Err(format!("structural audit: unknown reduction operation {operation:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> String {
        // The minimal shape the audit accepts, mirroring the emitter's text.
        let mut s = String::from(
            "var<workgroup> x: u32;\n\
             @group(0) @binding(0) var<storage, read> input0: array<u32>;\n\
             @group(0) @binding(1) var<storage, read_write> output0: array<u32>;\n\
             var<workgroup> scratch: array<u32, 64>;\n\
             fn mapper(p0: u32) -> u32 { return p0; }\n\
             @compute @workgroup_size(64)\nfn map_sum(a: u32, b: u32) {\n\
             \x20   let lane = a;\n    let i = b;\n\
             \x20   if (i >= arrayLength(&input0)) {\n\
             \x20       scratch[lane] = 0u;\n    } else {\n\
             \x20       scratch[lane] = mapper(input0[i]);\n    }\n\
             \x20   workgroupBarrier();\n",
        );
        s.push_str("if (lane < 32u) { scratch[lane] = scratch[lane] + scratch[lane + 32u]; } workgroupBarrier();\n");
        for stride in [16, 8, 4, 2, 1] {
            s.push_str(&format!(
                "if (lane < {stride}u) {{ scratch[lane] = scratch[lane] + scratch[lane + {stride}u]; }} workgroupBarrier();\n"
            ));
        }
        s.push_str("if (lane == 0u) { output0[wid.x] = scratch[0u]; }\n}\n");
        s.push_str(
            "@compute @workgroup_size(64)\nfn add_partials(a: u32, b: u32) {\n\
             \x20   let lane = a;\n    let i = b;\n\
             \x20   if (i >= arrayLength(&input0)) {\n\
             \x20       scratch[lane] = 0u;\n    } else {\n\
             \x20       scratch[lane] = input0[i];\n    }\n\
             \x20   workgroupBarrier();\n",
        );
        for stride in [32, 16, 8, 4, 2, 1] {
            s.push_str(&format!(
                "if (lane < {stride}u) {{ scratch[lane] = scratch[lane] + scratch[lane + {stride}u]; }} workgroupBarrier();\n"
            ));
        }
        s.push_str("if (lane == 0u) { output0[wid.x] = scratch[0u]; }\n}\n");
        s
    }

    fn good_x4() -> String {
        let mut s = String::from(
            "@group(0) @binding(0) var<storage, read> input0: array<u32>;\n\
             @group(0) @binding(1) var<storage, read_write> output0: array<u32>;\n\
             var<workgroup> scratch: array<u32, 64>;\n\
             fn mapper(p0: u32) -> u32 { return p0; }\n\
             @compute @workgroup_size(64)\nfn map_sum(a: u32, b: u32) {\n\
             \x20   let lane = a;\n    let i0 = wid.x * 256u + lane;\n\
             \x20   let i1 = i0 + 64u;\n    let i2 = i0 + 128u;\n    let i3 = i0 + 192u;\n\
             \x20   var acc: u32 = 0u;\n\
             \x20   if (i0 < arrayLength(&input0)) { acc = acc + mapper(input0[i0]); }\n\
             \x20   if (i1 < arrayLength(&input0)) { acc = acc + mapper(input0[i1]); }\n\
             \x20   if (i2 < arrayLength(&input0)) { acc = acc + mapper(input0[i2]); }\n\
             \x20   if (i3 < arrayLength(&input0)) { acc = acc + mapper(input0[i3]); }\n\
             \x20   scratch[lane] = acc;\n    workgroupBarrier();\n",
        );
        for stride in [32, 16, 8, 4, 2, 1] {
            s.push_str(&format!(
                "if (lane < {stride}u) {{ scratch[lane] = scratch[lane] + scratch[lane + {stride}u]; }} workgroupBarrier();\n"
            ));
        }
        s.push_str("if (lane == 0u) { output0[wid.x] = scratch[0u]; }\n}\n");
        s.push_str(
            "@compute @workgroup_size(64)\nfn add_partials(a: u32, b: u32) {\n\
             \x20   let lane = a;\n    let i0 = wid.x * 256u + lane;\n\
             \x20   let i1 = i0 + 64u;\n    let i2 = i0 + 128u;\n    let i3 = i0 + 192u;\n\
             \x20   var acc: u32 = 0u;\n\
             \x20   if (i0 < arrayLength(&input0)) { acc = acc + input0[i0]; }\n\
             \x20   if (i1 < arrayLength(&input0)) { acc = acc + input0[i1]; }\n\
             \x20   if (i2 < arrayLength(&input0)) { acc = acc + input0[i2]; }\n\
             \x20   if (i3 < arrayLength(&input0)) { acc = acc + input0[i3]; }\n\
             \x20   scratch[lane] = acc;\n    workgroupBarrier();\n",
        );
        for stride in [32, 16, 8, 4, 2, 1] {
            s.push_str(&format!(
                "if (lane < {stride}u) {{ scratch[lane] = scratch[lane] + scratch[lane + {stride}u]; }} workgroupBarrier();\n"
            ));
        }
        s.push_str("if (lane == 0u) { output0[wid.x] = scratch[0u]; }\n}\n");
        s
    }

    #[test]
    fn accepts_the_emitted_shape() {
        audit(&good()).unwrap();
    }

    #[test]
    fn accepts_the_x4_emitted_shape() {
        audit_x4(&good_x4()).unwrap();
    }

    #[test]
    fn x4_refuses_a_removed_chunk_guard() {
        let broken = good_x4().replacen("if (i2 < arrayLength(&input0))", "if (true)", 1);
        assert!(audit_x4(&broken).unwrap_err().contains("chunk 2"));
    }

    #[test]
    fn x4_refuses_a_missing_mapped_chunk() {
        let broken = good_x4().replacen(
            "acc = acc + mapper(input0[i3]);",
            "acc = acc + input0[i3];",
            1,
        );
        assert!(audit_x4(&broken).unwrap_err().contains("mapped load"));
    }

    #[test]
    fn x4_refuses_a_wrong_base_index() {
        let broken = good_x4().replacen("wid.x * 256u + lane", "wid.x * 64u + lane", 1);
        assert!(audit_x4(&broken).unwrap_err().contains("base index"));
    }

    #[test]
    fn x4_refuses_a_skipped_barrier() {
        let broken = good_x4().replacen("workgroupBarrier();", "", 1);
        assert!(audit_x4(&broken).unwrap_err().contains("barriers"));
    }

    #[test]
    fn x4_refuses_a_wrong_stride() {
        let broken = good_x4().replacen("if (lane < 16u)", "if (lane < 24u)", 1);
        assert!(audit_x4(&broken).unwrap_err().contains("stride"));
    }

    #[test]
    fn x4_refuses_a_non_lane_zero_writer() {
        let broken = good_x4().replacen("if (lane == 0u)", "if (lane == 1u)", 1);
        assert!(audit_x4(&broken).unwrap_err().contains("group writer"));
    }

    #[test]
    fn the_two_audits_reject_each_others_shape() {
        assert!(audit(&good_x4()).is_err(), "lane1 audit accepted x4");
        assert!(audit_x4(&good()).is_err(), "x4 audit accepted lane1");
        assert!(audit_for("map-sum-u32-nope", &good()).is_err());
    }

    #[test]
    fn refuses_a_removed_load_guard() {
        let broken = good().replacen("if (i >= arrayLength(&input0))", "if (false)", 1);
        assert!(audit(&broken).unwrap_err().contains("load guard"));
    }

    #[test]
    fn refuses_a_skipped_barrier() {
        let broken = good().replacen("workgroupBarrier();", "", 1);
        assert!(audit(&broken).unwrap_err().contains("barriers"));
    }

    #[test]
    fn refuses_a_wrong_stride() {
        let broken = good().replacen("if (lane < 16u)", "if (lane < 24u)", 1);
        assert!(audit(&broken).unwrap_err().contains("stride"));
    }

    #[test]
    fn refuses_a_non_lane_zero_writer() {
        let broken = good().replacen("if (lane == 0u)", "if (lane == 1u)", 1);
        assert!(audit(&broken).unwrap_err().contains("group writer"));
    }

    #[test]
    fn refuses_a_per_lane_group_write() {
        let broken = good().replacen("output0[wid.x] = scratch[0u];", "output0[lane] = scratch[0u];", 1);
        assert!(audit(&broken).unwrap_err().contains("per-lane output write"));
    }

    #[test]
    fn refuses_a_swapped_binding_role() {
        let broken = good().replacen("var<storage, read> input0", "var<storage, read_write> input0", 1);
        assert!(audit(&broken).unwrap_err().contains("input binding"));
    }
}
