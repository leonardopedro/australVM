//! G22c: ordered dispatch of the reduction.
//!
//! Preflight is independent of wgpu so the CPU-only build can test every
//! refusal: it derives the pass schedule from the validated input, checks each
//! pass's input and output extents against the adapter's limits, and never
//! takes a count from the artifact. The device path creates one pipeline per
//! entry from the single validated module, binds each pass's input to the
//! previous pass's output, submits the passes in order, and reads the final
//! word once. Intermediate buffers are retained until the readback so the
//! comparison artifacts stay available for a failing run.

use crate::reduce::{self, LaunchPlan, Strategy};
use crate::run::LimitsView;
use crate::schema::KernelIr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub plan: LaunchPlan,
    /// Workgroups per pass; `counts[pass]`, kept explicit for dispatch.
    pub groups: Vec<u32>,
    /// Bytes of each pass's output binding.
    pub bytes: Vec<u64>,
    pub total_bytes: u64,
}

/// Everything decidable before allocating device buffers. The layout comes
/// from the artifact's operation, never from a per-run declaration.
pub fn preflight(
    strategy: Strategy,
    n: usize,
    limits: &LimitsView,
) -> Result<Preflight, String> {
    let plan = strategy.derive_plan(n).map_err(|e| e.to_string())?;
    let mut groups = Vec::with_capacity(plan.counts.len());
    let mut bytes = Vec::with_capacity(plan.counts.len());
    let mut total: u64 = 0;
    for (pass, count) in plan.counts.iter().enumerate() {
        let extent = (*count as u64)
            .checked_mul(reduce::BYTES_PER_ELEMENT as u64)
            .ok_or_else(|| format!("pass {pass} extent overflow"))?;
        if extent > limits.max_buffer_size {
            return Err(format!(
                "pass {pass} output of {extent} bytes exceeds the adapter maximum {}",
                limits.max_buffer_size
            ));
        }
        if extent > limits.max_storage_buffer_binding_size {
            return Err(format!(
                "pass {pass} binding of {extent} bytes exceeds the adapter maximum {}",
                limits.max_storage_buffer_binding_size
            ));
        }
        if pass == 0 {
            let input = (n as u64)
                .checked_mul(reduce::BYTES_PER_ELEMENT as u64)
                .ok_or_else(|| "input extent overflow".to_string())?;
            if input > limits.max_buffer_size || input > limits.max_storage_buffer_binding_size {
                return Err(format!(
                    "input binding of {input} bytes exceeds the adapter limits"
                ));
            }
            total = total
                .checked_add(input)
                .ok_or_else(|| "total allocation overflow".to_string())?;
        }
        if *count as u64 > limits.max_compute_workgroups_per_dimension as u64 {
            return Err(format!(
                "pass {pass} dispatch of {count} workgroups exceeds the adapter maximum {}",
                limits.max_compute_workgroups_per_dimension
            ));
        }
        total = total
            .checked_add(extent)
            .ok_or_else(|| "total allocation overflow".to_string())?;
        groups.push(*count as u32);
        bytes.push(extent);
    }
    if reduce::TILE as u32 > limits.max_compute_workgroup_size_x {
        return Err(format!(
            "workgroup size {} exceeds the adapter maximum {}",
            reduce::TILE,
            limits.max_compute_workgroup_size_x
        ));
    }
    Ok(Preflight {
        plan,
        groups,
        bytes,
        total_bytes: total,
    })
}

/// The CPU oracle for a loaded reduction: evaluate the mapper on each input
/// word, then apply the same tiled schedule the device executes.
pub fn cpu_reduce(strategy: Strategy, ir: &KernelIr, input: &[u32]) -> Result<u32, String> {
    let mut evaluator = crate::reference::Evaluator::new(ir, Default::default())?;
    let mut mapped = Vec::with_capacity(input.len());
    for value in input {
        let out = evaluator.run(&[vec![*value]])?;
        if out.len() != 1 {
            return Err(format!("mapper returned {} values", out.len()));
        }
        mapped.push(out[0]);
    }
    // `tiled_map_sum` applies the mapper itself; feed it the already-mapped
    // values through the identity so the schedule is exercised once.
    reduce::tiled_map_sum_for(strategy, |x| x, &mapped).map_err(|e| e.to_string())
}

#[cfg(feature = "device")]
mod device_path {
    use super::*;
    use crate::device::{self, AdapterEvidence, ProbeError};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    pub struct ReduceOutput {
        pub adapter: AdapterEvidence,
        pub output: u32,
        pub preflight: Preflight,
        pub dispatch_count: u32,
        pub iterations: u32,
        pub prepare_ms: f64,
        pub upload_ms: f64,
        pub submit_ms: f64,
        pub readback_ms: f64,
    }

    #[derive(Debug)]
    pub enum ReduceFailure {
        Unavailable(String),
        Refused(String),
    }

    fn unavailable_or_refused(e: ProbeError) -> ReduceFailure {
        match e {
            ProbeError::Unavailable(m) => ReduceFailure::Unavailable(m),
            ProbeError::NotPhysical(_) => {
                ReduceFailure::Refused("adapter reports a non-physical device type".to_string())
            }
            ProbeError::Failed(m) => ReduceFailure::Refused(m),
        }
    }

    pub fn run(
        loaded: &crate::LoadedReduction,
        input: &[u32],
        require_hardware: bool,
        selection: &device::Selection,
    ) -> Result<ReduceOutput, ReduceFailure> {
        run_with_iterations(loaded, input, require_hardware, selection, 1)
    }

    /// Like [`run`], but submits the ordered pass sequence `iterations` times
    /// after one preparation, so later submissions reuse the pipelines and
    /// buffers. Only the last sequence is read back; `submit_ms` is the total.
    pub fn run_with_iterations(
        loaded: &crate::LoadedReduction,
        input: &[u32],
        require_hardware: bool,
        selection: &device::Selection,
        iterations: u32,
    ) -> Result<ReduceOutput, ReduceFailure> {
        let (adapter, evidence) =
            device::request(selection).map_err(unavailable_or_refused)?;
        if require_hardware && !evidence.physical {
            return Err(ReduceFailure::Refused(format!(
                "adapter is {}, not a physical GPU",
                evidence.device_type
            )));
        }
        let limits = LimitsView {
            max_buffer_size: evidence.max_buffer_size,
            max_storage_buffer_binding_size: evidence.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: evidence.max_compute_workgroups_per_dimension,
            max_compute_workgroup_size_x: evidence.max_workgroup_size_x,
        };
        let iterations = iterations.max(1);
        let plan = preflight(loaded.strategy(), input.len(), &limits)
            .map_err(ReduceFailure::Refused)?;

        let device_desc = wgpu::DeviceDescriptor {
            label: Some("avm-gpu reduce"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        };
        let prepare_start = Instant::now();
        let (device, queue) = pollster::block_on(adapter.request_device(&device_desc))
            .map_err(|e| ReduceFailure::Refused(format!("request-device failed: {e}")))?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("avm-gpu reduce module"),
            source: wgpu::ShaderSource::Wgsl(loaded.wgsl.as_str().into()),
        });
        let mapper_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("avm-gpu map_sum"),
            layout: None,
            module: &module,
            entry_point: Some(crate::reduce_artifact::MAPPER_ENTRY),
            compilation_options: Default::default(),
            cache: None,
        });
        let partial_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("avm-gpu add_partials"),
            layout: None,
            module: &module,
            entry_point: Some(crate::reduce_artifact::PARTIAL_ENTRY),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(ReduceFailure::Refused(format!(
                "reduction shader/pipeline validation failed: {error}"
            )));
        }
        let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;

        if plan.groups.is_empty() {
            return Ok(ReduceOutput {
                adapter: evidence,
                output: reduce::IDENTITY,
                preflight: plan,
                dispatch_count: 0,
                iterations,
                prepare_ms,
                upload_ms: 0.0,
                submit_ms: 0.0,
                readback_ms: 0.0,
            });
        }

        let upload_start = Instant::now();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let input_bytes: Vec<u8> = input.iter().flat_map(|v| v.to_le_bytes()).collect();
        let input_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("avm-gpu reduce input"),
            size: input_bytes.len() as u64,
            usage: storage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&input_buffer, 0, &input_bytes);
        let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;

        // Every pass gets its own output buffer; the previous pass's buffer is
        // the next pass's input, so no pass overwrites storage it still reads.
        let mut intermediates: Vec<wgpu::Buffer> = Vec::new();
        let mut bind_groups: Vec<(usize, wgpu::BindGroup)> = Vec::new();
        let mut current = input_buffer;
        let mut dispatch_count = 0u32;
        for pass in 0..plan.groups.len() {
            let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("avm-gpu reduce pass{pass}")),
                size: plan.bytes[pass],
                usage: storage,
                mapped_at_creation: false,
            });
            let pipeline = if pass == 0 {
                &mapper_pipeline
            } else {
                &partial_pipeline
            };
            let layout = pipeline.get_bind_group_layout(0);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("avm-gpu reduce bind{pass}")),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: current.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output_buffer.as_entire_binding(),
                    },
                ],
            });
            current = output_buffer.clone();
            intermediates.push(output_buffer);
            bind_groups.push((pass, bind_group));
            dispatch_count += 1;
        }

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("avm-gpu reduce staging"),
            size: reduce::BYTES_PER_ELEMENT as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let pipelines = [&mapper_pipeline, &partial_pipeline];
        let submit_start = Instant::now();
        let mut submission = None;
        for _ in 0..iterations {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            for (pass, bind_group) in &bind_groups {
                let pipeline = if *pass == 0 { pipelines[0] } else { pipelines[1] };
                let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(if *pass == 0 {
                        "avm-gpu map_sum"
                    } else {
                        "avm-gpu add_partials"
                    }),
                    timestamp_writes: None,
                });
                compute.set_pipeline(pipeline);
                compute.set_bind_group(0, bind_group, &[]);
                compute.dispatch_workgroups(plan.groups[*pass], 1, 1);
            }
            encoder.copy_buffer_to_buffer(
                &current,
                0,
                &staging,
                0,
                reduce::BYTES_PER_ELEMENT as u64,
            );
            submission = Some(queue.submit(Some(encoder.finish())));
        }
        let submission = submission.expect("iterations is at least one");
        let submit_ms = submit_start.elapsed().as_secs_f64() * 1000.0;

        let readback_start = Instant::now();
        let (sender, receiver) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        match device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        }) {
            Ok(_) => {}
            Err(wgpu::PollError::Timeout) => {
                return Err(ReduceFailure::Refused(
                    "reduction poll timed out after 30s; buffers retained, not reused"
                        .to_string(),
                ))
            }
            Err(e) => {
                return Err(ReduceFailure::Refused(format!(
                    "reduction device poll failed: {e}; buffers retained, not reused"
                )))
            }
        }
        match receiver.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(ReduceFailure::Refused(format!(
                    "final word mapping failed: {e}"
                )))
            }
            Err(e) => {
                return Err(ReduceFailure::Refused(format!(
                    "final word mapping callback failed: {e}"
                )))
            }
        }
        let output = {
            let view = staging.slice(..).get_mapped_range();
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&view);
            drop(view);
            u32::from_le_bytes(bytes)
        };
        staging.unmap();
        let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
        drop(intermediates);
        Ok(ReduceOutput {
            adapter: evidence,
            output,
            preflight: plan,
            dispatch_count: dispatch_count.saturating_mul(iterations),
            iterations,
            prepare_ms,
            upload_ms,
            submit_ms,
            readback_ms,
        })
    }
}

#[cfg(feature = "device")]
pub use device_path::{run, run_with_iterations, ReduceFailure, ReduceOutput};

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> LimitsView {
        LimitsView {
            max_buffer_size: 1 << 32,
            max_storage_buffer_binding_size: 1 << 32,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_workgroup_size_x: 256,
        }
    }

    #[test]
    fn preflight_derives_the_schedule() {
        let one = Strategy::OnePerLane;
        let empty = preflight(one, 0, &limits()).unwrap();
        assert!(empty.groups.is_empty());
        assert_eq!(empty.total_bytes, 0);

        let p = preflight(one, 65, &limits()).unwrap();
        assert_eq!(p.groups, vec![2, 1]);
        assert_eq!(p.bytes, vec![8, 4]);
        assert_eq!(p.total_bytes, 65 * 4 + 8 + 4);

        let p = preflight(one, 1_048_576, &limits()).unwrap();
        assert_eq!(p.groups, vec![16_384, 256, 4, 1]);

        let four = Strategy::FourPerLane;
        assert!(preflight(four, 0, &limits()).unwrap().groups.is_empty());
        let p = preflight(four, 257, &limits()).unwrap();
        assert_eq!(p.groups, vec![2, 1]);
        let p = preflight(four, 1_048_576, &limits()).unwrap();
        assert_eq!(p.groups, vec![4096, 16, 1]);
    }

    #[test]
    fn preflight_refuses_each_limit() {
        let one = Strategy::OnePerLane;
        let mut l = limits();
        l.max_buffer_size = 8;
        assert!(preflight(one, 65, &l).unwrap_err().contains("exceeds"));

        let mut l = limits();
        l.max_storage_buffer_binding_size = 8;
        assert!(preflight(one, 65, &l).unwrap_err().contains("exceeds"));

        let mut l = limits();
        l.max_compute_workgroups_per_dimension = 1;
        assert!(preflight(one, 65, &l).unwrap_err().contains("workgroups"));

        let mut l = limits();
        l.max_compute_workgroup_size_x = 32;
        assert!(preflight(one, 1, &l).unwrap_err().contains("workgroup size"));

        assert!(preflight(one, reduce::MAX_INPUTS + 1, &limits()).is_err());
    }

    #[test]
    fn cpu_reduce_matches_the_serial_oracle() {
        use crate::schema::{Expr, MapKind, MemoryPlan, Stmt, NUMERIC_PROFILE};
        let ir = KernelIr {
            entry: "main".to_string(),
            input_arity: 1,
            helpers: Vec::new(),
            body: Stmt::Return {
                value: Expr::Intrinsic {
                    op: "add".to_string(),
                    args: vec![
                        Expr::Intrinsic {
                            op: "multiply".to_string(),
                            args: vec![
                                Expr::Param { index: 0 },
                                Expr::U32 { value: "3".to_string() },
                            ],
                        },
                        Expr::U32 { value: "1".to_string() },
                    ],
                },
            },
            memory: MemoryPlan {
                map: MapKind::Map1,
                output_index: "identity".to_string(),
                guard_before_load: true,
                workgroup_size: 64,
            },
            numeric_profile: NUMERIC_PROFILE.to_string(),
        };
        for n in [0usize, 1, 63, 64, 65, 4097] {
            let input: Vec<u32> = (0..n)
                .map(|i| (i as u32).wrapping_mul(2_654_435_761).wrapping_add(12_345))
                .collect();
            for strategy in [Strategy::OnePerLane, Strategy::FourPerLane] {
                assert_eq!(
                    cpu_reduce(strategy, &ir, &input),
                    Ok(reduce::serial_map_sum(|x| x.wrapping_mul(3).wrapping_add(1), &input)),
                    "cpu oracle disagrees at {n} ({strategy:?})"
                );
            }
        }
    }
}
