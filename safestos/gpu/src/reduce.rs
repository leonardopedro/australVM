//! G22a: `map-sum-u32` — the fixed reduction's definition and CPU schedule.
//!
//! The operation is one `u32 -> u32` mapper (from the Austral source) and an
//! explicit wrapping sum modulo 2^32, with identity 0 for empty input. This
//! module is the CPU side the WGSL emitter (G22b) and the device runner (G22c)
//! have to agree with: the pass counts, the `(group, lane)` partition and the
//! per-pass plan are derived here, once, from the validated input length.
//!
//! A pass over `n > 0` elements produces `ceil(n/64)` outputs; the partition is
//! `i = 64 * group + lane` with `0 <= lane < 64`. A short tail contributes the
//! identity: its missing lanes are never mapped. That is why the oracle mapper
//! has `f(0) != 0` — an implementation that maps padding instead of adding the
//! identity changes the sum.

use std::fmt;

pub const OPERATION: &str = "map-sum-u32";
pub const PROFILE: &str = "reduce-u32-v1";
pub const TILE: usize = 64;
pub const IDENTITY: u32 = 0;
pub const MAX_INPUTS: usize = 1_048_576;
pub const BYTES_PER_ELEMENT: usize = 4;
/// One workgroup's scratch: `TILE` u32 words, independent of input length.
pub const SCRATCH_BYTES: usize = TILE * BYTES_PER_ELEMENT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    OversizedInput(usize),
    CapacityOverflow(&'static str),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::OversizedInput(n) => write!(
                f,
                "input count {n} exceeds the {MAX_INPUTS}-element v1 maximum"
            ),
            ScheduleError::CapacityOverflow(what) => {
                write!(f, "{what} does not fit in a host usize")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

fn ceil_div(a: usize, b: usize) -> usize {
    a.div_ceil(b)
}

/// Output count of each pass, pass 0 first; empty for empty input.
pub fn schedule(n: usize) -> Result<Vec<usize>, ScheduleError> {
    if n > MAX_INPUTS {
        return Err(ScheduleError::OversizedInput(n));
    }
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut counts = vec![ceil_div(n, TILE)];
    while let Some(&out) = counts.last() {
        if out <= 1 {
            break;
        }
        counts.push(ceil_div(out, TILE));
    }
    Ok(counts)
}

pub fn pass_count(n: usize) -> usize {
    schedule(n).map(|counts| counts.len()).unwrap_or(0)
}

/// Byte capacity of `elements` u32 words, refusing an oversized count.
pub fn capacity_bytes(elements: usize) -> Result<usize, ScheduleError> {
    if elements > MAX_INPUTS {
        return Err(ScheduleError::OversizedInput(elements));
    }
    elements
        .checked_mul(BYTES_PER_ELEMENT)
        .ok_or(ScheduleError::CapacityOverflow("element bytes"))
}

/// The caller's input plus every pass's output: distinct storage per pass.
pub fn total_bytes(n: usize) -> Result<usize, ScheduleError> {
    let input = capacity_bytes(n)?;
    let outputs: usize = schedule(n)?
        .iter()
        .try_fold(0usize, |acc, out| acc.checked_add(*out))
        .ok_or(ScheduleError::CapacityOverflow("pass outputs"))?;
    let output_bytes = outputs
        .checked_mul(BYTES_PER_ELEMENT)
        .ok_or(ScheduleError::CapacityOverflow("pass output bytes"))?;
    input
        .checked_add(output_bytes)
        .ok_or(ScheduleError::CapacityOverflow("total allocation"))
}

/// The only index a `(group, lane)` pair names; `None` outside a workgroup.
pub fn partition_index(group: usize, lane: usize) -> Option<usize> {
    if lane >= TILE {
        return None;
    }
    Some(TILE * group + lane)
}

/// Independent serial oracle: map each element, then fold with wrapping add.
pub fn serial_map_sum<F: Fn(u32) -> u32>(mapper: F, input: &[u32]) -> u32 {
    input
        .iter()
        .fold(IDENTITY, |acc, x| acc.wrapping_add(mapper(*x)))
}

/// The tiled schedule: pass 0 maps and partially sums, later passes only sum
/// the previous pass's partials until one remains. Short tails add the
/// identity; padding is never mapped.
pub fn tiled_map_sum<F: Fn(u32) -> u32>(
    mapper: F,
    input: &[u32],
) -> Result<u32, ScheduleError> {
    let counts = schedule(input.len())?;
    if counts.is_empty() {
        return Ok(IDENTITY);
    }
    let mut current: Vec<u32> = Vec::with_capacity(counts[0]);
    for group in 0..counts[0] {
        let mut acc = IDENTITY;
        for lane in 0..TILE {
            let i = group * TILE + lane;
            if i < input.len() {
                acc = acc.wrapping_add(mapper(input[i]));
            }
        }
        current.push(acc);
    }
    while current.len() > 1 {
        let mut next: Vec<u32> = Vec::with_capacity(ceil_div(current.len(), TILE));
        for group in 0..ceil_div(current.len(), TILE) {
            let mut acc = IDENTITY;
            for lane in 0..TILE {
                let i = group * TILE + lane;
                if i < current.len() {
                    acc = acc.wrapping_add(current[i]);
                }
            }
            next.push(acc);
        }
        current = next;
    }
    Ok(current[0])
}

/// What a pass's input binding is: pass 0 reads the mapper's input, every later
/// pass reads a previous pass's partial sums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    MapperInput,
    PartialInput,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::MapperInput => f.write_str("mapper-input"),
            Role::PartialInput => f.write_str("partial-input"),
        }
    }
}

/// A launch plan as an artifact would declare it. The host derives the same
/// values from the validated input and refuses a mismatch; it never accepts
/// counts or extents from the artifact (`G22a`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub counts: Vec<usize>,
    pub extents: Vec<usize>,
    pub identity: u32,
    pub roles: Vec<Role>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Schedule(ScheduleError),
    PassCountMismatch { declared: usize, derived: usize },
    CountMismatch { pass: usize, declared: usize, derived: usize },
    ExtentMismatch { pass: usize, declared: usize, derived: usize },
    IdentityMismatch { declared: u32 },
    RoleMismatch { pass: usize, declared: Role },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::Schedule(e) => write!(f, "{e}"),
            PlanError::PassCountMismatch { declared, derived } => write!(
                f,
                "artifact declares {declared} pass(es), the input derives {derived}"
            ),
            PlanError::CountMismatch {
                pass,
                declared,
                derived,
            } => write!(
                f,
                "pass {pass} declares {declared} output(s), the schedule derives {derived}"
            ),
            PlanError::ExtentMismatch {
                pass,
                declared,
                derived,
            } => write!(
                f,
                "pass {pass} declares {declared} output byte(s), the schedule derives {derived}"
            ),
            PlanError::IdentityMismatch { declared } => write!(
                f,
                "artifact declares identity {declared}, the operation's is {IDENTITY}"
            ),
            PlanError::RoleMismatch { pass, declared } => write!(
                f,
                "pass {pass} declares role {declared}, the schedule derives {}",
                if *pass == 0 { Role::MapperInput } else { Role::PartialInput }
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// The plan the host derives for `n` validated inputs.
pub fn derive_plan(n: usize) -> Result<LaunchPlan, ScheduleError> {
    let counts = schedule(n)?;
    let roles = (0..counts.len())
        .map(|pass| if pass == 0 { Role::MapperInput } else { Role::PartialInput })
        .collect();
    let extents = counts
        .iter()
        .map(|out| capacity_bytes(*out))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LaunchPlan {
        counts,
        extents,
        identity: IDENTITY,
        roles,
    })
}

/// Refuses a declared plan that disagrees with the derived one.
pub fn validate_plan(n: usize, plan: &LaunchPlan) -> Result<(), PlanError> {
    let derived = derive_plan(n).map_err(PlanError::Schedule)?;
    if plan.identity != derived.identity {
        return Err(PlanError::IdentityMismatch {
            declared: plan.identity,
        });
    }
    if plan.counts.len() != derived.counts.len() {
        return Err(PlanError::PassCountMismatch {
            declared: plan.counts.len(),
            derived: derived.counts.len(),
        });
    }
    for (pass, (declared, expected)) in plan.counts.iter().zip(&derived.counts).enumerate() {
        if declared != expected {
            return Err(PlanError::CountMismatch {
                pass,
                declared: *declared,
                derived: *expected,
            });
        }
    }
    if plan.extents.len() != derived.extents.len() {
        return Err(PlanError::PassCountMismatch {
            declared: plan.extents.len(),
            derived: derived.extents.len(),
        });
    }
    for (pass, (declared, expected)) in plan.extents.iter().zip(&derived.extents).enumerate() {
        if declared != expected {
            return Err(PlanError::ExtentMismatch {
                pass,
                declared: *declared,
                derived: *expected,
            });
        }
    }
    if plan.roles.len() != derived.roles.len() {
        return Err(PlanError::PassCountMismatch {
            declared: plan.roles.len(),
            derived: derived.roles.len(),
        });
    }
    for (pass, (declared, expected)) in plan.roles.iter().zip(&derived.roles).enumerate() {
        if declared != expected {
            return Err(PlanError::RoleMismatch {
                pass,
                declared: *declared,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The oracle mapper: `f(x) = 3x + 1` (wrapping), so `f(0) = 1 != 0`.
    fn mapper(x: u32) -> u32 {
        x.wrapping_mul(3).wrapping_add(1)
    }

    /// The shared deterministic input, documented in `docs/gpu/REDUCE_U32.md`.
    fn value(i: usize) -> u32 {
        (i as u32)
            .wrapping_mul(2_654_435_761)
            .wrapping_add(12_345)
    }

    fn input(n: usize) -> Vec<u32> {
        (0..n).map(value).collect()
    }

    const SIZES: [usize; 9] = [
        0, 1, 63, 64, 65, 4_095, 4_096, 4_097, 1_048_576,
    ];

    #[test]
    fn schedule_matches_the_named_sizes() {
        let cases: [(usize, &[usize]); 9] = [
            (0, &[]),
            (1, &[1]),
            (63, &[1]),
            (64, &[1]),
            (65, &[2, 1]),
            (4_095, &[64, 1]),
            (4_096, &[64, 1]),
            (4_097, &[65, 2, 1]),
            (1_048_576, &[16_384, 256, 4, 1]),
        ];
        for (n, expected) in cases {
            assert_eq!(schedule(n).unwrap(), expected, "schedule for {n}");
        }
    }

    #[test]
    fn at_most_four_passes() {
        assert_eq!(pass_count(MAX_INPUTS), 4);
        for n in SIZES {
            assert!(pass_count(n) <= 4, "{n} needs more than four passes");
        }
    }

    #[test]
    fn refusals() {
        assert_eq!(
            schedule(MAX_INPUTS + 1),
            Err(ScheduleError::OversizedInput(MAX_INPUTS + 1))
        );
        assert_eq!(
            capacity_bytes(MAX_INPUTS + 1),
            Err(ScheduleError::OversizedInput(MAX_INPUTS + 1))
        );
        assert!(matches!(
            total_bytes(MAX_INPUTS + 1),
            Err(ScheduleError::OversizedInput(_))
        ));
    }

    #[test]
    fn capacities() {
        assert_eq!(capacity_bytes(0), Ok(0));
        assert_eq!(capacity_bytes(1), Ok(4));
        assert_eq!(capacity_bytes(TILE), Ok(256));
        assert_eq!(capacity_bytes(MAX_INPUTS), Ok(4_194_304));
        assert_eq!(
            total_bytes(MAX_INPUTS),
            Ok(4_194_304 + (16_384 + 256 + 4 + 1) * 4)
        );
        assert_eq!(SCRATCH_BYTES, 256);
    }

    #[test]
    fn empty_input_is_the_identity_without_mapping() {
        let calls = std::cell::Cell::new(0usize);
        let counting = |x: u32| {
            calls.set(calls.get() + 1);
            mapper(x)
        };
        assert_eq!(tiled_map_sum(counting, &[]), Ok(IDENTITY));
        assert_eq!(calls.get(), 0, "the mapper ran on an empty input");
    }

    #[test]
    fn tiled_agrees_with_the_serial_oracle() {
        for n in SIZES {
            let data = input(n);
            assert_eq!(
                tiled_map_sum(mapper, &data),
                Ok(serial_map_sum(mapper, &data)),
                "schedule disagrees with the serial sum at {n}"
            );
        }
    }

    /// Padding must get the identity, not `mapper(0)`.
    #[test]
    fn mapping_padding_would_change_the_sum() {
        for n in [63usize, 65, 4_095, 4_097] {
            let data = input(n);
            let buggy: u32 = (0..ceil_div(n, TILE))
                .map(|group| {
                    (0..TILE).fold(IDENTITY, |acc, lane| {
                        let i = group * TILE + lane;
                        let v = if i < n { mapper(data[i]) } else { mapper(0) };
                        acc.wrapping_add(v)
                    })
                })
                .fold(IDENTITY, u32::wrapping_add);
            assert_ne!(
                buggy,
                serial_map_sum(mapper, &data),
                "the padding control is vacuous at {n}: mapper(0) == 0"
            );
            assert_ne!(buggy, tiled_map_sum(mapper, &data).unwrap());
        }
    }

    #[test]
    fn partition_covers_each_index_once() {
        for n in SIZES {
            let mut seen = vec![0usize; n];
            for group in 0..ceil_div(n, TILE) {
                for lane in 0..TILE {
                    if let Some(i) = partition_index(group, lane) {
                        if i < n {
                            seen[i] += 1;
                        }
                    }
                }
            }
            assert!(
                seen.iter().all(|count| *count == 1),
                "partition misses or repeats an index at {n}"
            );
        }
        assert_eq!(partition_index(0, 0), Some(0));
        assert_eq!(partition_index(0, 63), Some(63));
        assert_eq!(partition_index(1, 0), Some(64));
        assert_eq!(partition_index(0, 64), None);
    }

    #[test]
    fn plan_round_trips_and_refuses_each_field() {
        for n in SIZES {
            let plan = derive_plan(n).unwrap();
            assert_eq!(validate_plan(n, &plan), Ok(()), "plan for {n}");
        }

        let mut wrong = derive_plan(65).unwrap();
        wrong.identity = 7;
        assert!(matches!(
            validate_plan(65, &wrong),
            Err(PlanError::IdentityMismatch { declared: 7 })
        ));

        let mut wrong = derive_plan(65).unwrap();
        wrong.counts[0] = 3;
        assert!(matches!(
            validate_plan(65, &wrong),
            Err(PlanError::CountMismatch { pass: 0, .. })
        ));

        let mut wrong = derive_plan(65).unwrap();
        wrong.counts.push(1);
        assert!(matches!(
            validate_plan(65, &wrong),
            Err(PlanError::PassCountMismatch { .. })
        ));

        let mut wrong = derive_plan(65).unwrap();
        wrong.extents[1] = 8;
        assert!(matches!(
            validate_plan(65, &wrong),
            Err(PlanError::ExtentMismatch { pass: 1, .. })
        ));

        let mut wrong = derive_plan(65).unwrap();
        wrong.roles[1] = Role::MapperInput;
        assert!(matches!(
            validate_plan(65, &wrong),
            Err(PlanError::RoleMismatch { pass: 1, .. })
        ));

        assert!(matches!(
            validate_plan(MAX_INPUTS + 1, &derive_plan(1).unwrap()),
            Err(PlanError::Schedule(ScheduleError::OversizedInput(_)))
        ));
    }

    /// The C oracle's pinned table (`scripts/gpu-reduce-u32`): the serial sum
    /// over the same generator must reproduce every row bit for bit.
    #[test]
    fn c_oracle_table_matches_the_serial_sum() {
        let table = include_str!("../../../examples/gpu/reduce/expected.tsv");
        let rows = table
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| {
                let (size, expected) = line.split_once('\t').expect("size\\texpected");
                (
                    size.parse::<usize>().expect("size"),
                    expected.parse::<u32>().expect("expected"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), SIZES.len(), "oracle table row count");
        for (n, expected) in rows {
            assert_eq!(
                serial_map_sum(mapper, &input(n)),
                expected,
                "serial oracle disagrees with the C table at {n}"
            );
            assert_eq!(
                tiled_map_sum(mapper, &input(n)),
                Ok(expected),
                "schedule disagrees with the C table at {n}"
            );
        }
    }
}
