//! G10: preflight and device execution for validated bundles.
//!
//! Preflight is deliberately independent of wgpu so the default (CPU-only)
//! build can test every refusal before a device exists. The device path asks
//! for an adapter, checks the requested buffers against its limits, creates
//! the buffers, submits exactly one dispatch, waits for completion, then maps
//! and reads a staging copy. It never evaluates the IR as a substitute for a
//! GPU result.

use crate::schema::{KernelBundle, MAX_ELEMENTS, WORKGROUP_SIZE};

/// The device facts preflight needs, kept separate from wgpu types.
#[derive(Debug, Clone, Copy)]
pub struct LimitsView {
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroups_per_dimension: u32,
    pub max_compute_workgroup_size_x: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preflight {
    pub n: usize,
    pub bytes: u64,
    pub groups: u32,
}

/// Everything that can be decided before allocating device buffers.
pub fn preflight(
    bundle: &KernelBundle,
    inputs: &[Vec<u32>],
    limits: &LimitsView,
) -> Result<Preflight, String> {
    let arity = bundle.input_arity as usize;
    if inputs.len() != arity {
        return Err(format!(
            "input arity is {}, the request carries {}",
            arity,
            inputs.len()
        ));
    }
    let n = inputs.first().map(Vec::len).unwrap_or(0);
    if inputs.iter().any(|i| i.len() != n) {
        return Err("input arrays have unequal lengths".to_string());
    }
    if n > MAX_ELEMENTS as usize {
        return Err(format!(
            "N = {n} exceeds the release limit {MAX_ELEMENTS}"
        ));
    }
    let bytes = (n as u64)
        .checked_mul(4)
        .ok_or_else(|| "input size overflow".to_string())?;
    if bytes > limits.max_buffer_size {
        return Err(format!(
            "buffer of {bytes} bytes exceeds the adapter maximum {}",
            limits.max_buffer_size
        ));
    }
    if bytes > limits.max_storage_buffer_binding_size {
        return Err(format!(
            "binding of {bytes} bytes exceeds the adapter maximum {}",
            limits.max_storage_buffer_binding_size
        ));
    }
    if WORKGROUP_SIZE > limits.max_compute_workgroup_size_x {
        return Err(format!(
            "workgroup size {WORKGROUP_SIZE} exceeds the adapter maximum {}",
            limits.max_compute_workgroup_size_x
        ));
    }
    let n64 = n as u64;
    let groups64 = n64 / WORKGROUP_SIZE as u64 + u64::from(n64 % WORKGROUP_SIZE as u64 != 0);
    if groups64 > limits.max_compute_workgroups_per_dimension as u64 {
        return Err(format!(
            "dispatch of {groups64} workgroups exceeds the adapter maximum {}",
            limits.max_compute_workgroups_per_dimension
        ));
    }
    Ok(Preflight {
        n,
        bytes,
        groups: groups64 as u32,
    })
}

#[cfg(feature = "device")]
mod device_path {
    use super::*;
    use crate::device::{self, AdapterEvidence, ProbeError};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    pub struct DeviceOutput {
        pub adapter: AdapterEvidence,
        pub output: Vec<u32>,
        pub preflight: Preflight,
        pub dispatch_count: u32,
        /// Adapter request, device creation and pipeline creation.
        pub prepare_ms: f64,
        /// Input buffer creation and `write_buffer` calls.
        pub upload_ms: f64,
        pub submit_ms: f64,
        pub readback_ms: f64,
    }

    #[derive(Debug)]
    pub enum RunFailure {
        /// No adapter at all; optional mode maps this to exit 77.
        Unavailable(String),
        /// The request or adapter was refused.
        Refused(String),
    }

    struct Errors(std::sync::Arc<std::sync::Mutex<Option<String>>>);

    pub fn run(
        loaded: &crate::LoadedBundle,
        inputs: &[Vec<u32>],
        require_hardware: bool,
        selection: &device::Selection,
    ) -> Result<DeviceOutput, RunFailure> {
        run_inner(loaded, inputs, require_hardware, selection, None)
    }

    /// Like [`run`], but the readback is also handed to `on_ready` from the
    /// queue's submitted-work-done callback. That callback is a runtime event,
    /// not a step after `run` returns: a caller that publishes a job's
    /// completion here is publishing it when the GPU says the submitted work
    /// and its staging copy are done.
    pub fn run_with_completion<F>(
        loaded: &crate::LoadedBundle,
        inputs: &[Vec<u32>],
        require_hardware: bool,
        selection: &device::Selection,
        on_ready: F,
    ) -> Result<DeviceOutput, RunFailure>
    where
        F: FnOnce(&[u32]) + Send + 'static,
    {
        run_inner(loaded, inputs, require_hardware, selection, Some(Box::new(on_ready)))
    }

    type CompletionHook = Box<dyn FnOnce(&[u32]) + Send>;

    fn run_inner(
        loaded: &crate::LoadedBundle,
        inputs: &[Vec<u32>],
        require_hardware: bool,
        selection: &device::Selection,
        on_ready: Option<CompletionHook>,
    ) -> Result<DeviceOutput, RunFailure> {
        let bundle = &loaded.bundle;
        let (adapter, evidence) = match device::request(selection) {
            Ok(pair) => pair,
            Err(ProbeError::Unavailable(message)) => {
                return Err(RunFailure::Unavailable(message))
            }
            Err(ProbeError::NotPhysical(_)) => {
                return Err(RunFailure::Refused(
                    "adapter reports a non-physical device type".to_string(),
                ))
            }
            Err(ProbeError::Failed(message)) => return Err(RunFailure::Refused(message)),
        };
        if require_hardware && !evidence.physical {
            return Err(RunFailure::Refused(format!(
                "adapter is {}, not a physical GPU",
                evidence.device_type
            )));
        }
        let limits = LimitsView {
            max_buffer_size: evidence.max_buffer_size,
            max_storage_buffer_binding_size: evidence.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: evidence
                .max_compute_workgroups_per_dimension,
            max_compute_workgroup_size_x: evidence.max_workgroup_size_x,
        };
        let plan = preflight(bundle, inputs, &limits).map_err(RunFailure::Refused)?;

        let timing = execute(loaded, inputs, &plan, &adapter, &evidence, on_ready)?;
        Ok(DeviceOutput {
            adapter: evidence,
            output: timing.output,
            preflight: plan,
            dispatch_count: timing.dispatch_count,
            prepare_ms: timing.prepare_ms,
            upload_ms: timing.upload_ms,
            submit_ms: timing.submit_ms,
            readback_ms: timing.readback_ms,
        })
    }

    struct Timing {
        output: Vec<u32>,
        dispatch_count: u32,
        prepare_ms: f64,
        upload_ms: f64,
        submit_ms: f64,
        readback_ms: f64,
    }

    fn execute(
        loaded: &crate::LoadedBundle,
        inputs: &[Vec<u32>],
        plan: &Preflight,
        adapter: &wgpu::Adapter,
        _evidence: &AdapterEvidence,
        on_ready: Option<CompletionHook>,
    ) -> Result<Timing, RunFailure> {
        let prepare_start = Instant::now();
        let device_desc = wgpu::DeviceDescriptor {
            label: Some("avm-gpu run"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        };
        let device_queue = pollster::block_on(adapter.request_device(&device_desc))
            .map_err(|e| RunFailure::Refused(format!("request-device failed: {e}")))?;
        let (device, queue) = device_queue;

        let lost: Errors = Errors(Default::default());
        let lost_clone = lost.0.clone();
        device.set_device_lost_callback(move |reason, message| {
            *lost_clone.lock().unwrap() = Some(format!("device lost ({reason:?}): {message}"));
        });

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("avm-gpu kernel"),
            source: wgpu::ShaderSource::Wgsl(loaded.wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("avm-gpu pipeline"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(RunFailure::Refused(format!(
                "shader/pipeline validation failed: {error}"
            )));
        }

        let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
        if plan.groups == 0 {
            return Ok(Timing {
                output: Vec::new(),
                dispatch_count: 0,
                prepare_ms,
                upload_ms: 0.0,
                submit_ms: 0.0,
                readback_ms: 0.0,
            });
        }
        let upload_start = Instant::now();

        let size = plan.bytes;
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("avm-gpu output"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("avm-gpu staging"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut input_buffers = Vec::new();
        for (i, values) in inputs.iter().enumerate() {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("avm-gpu input{i}")),
                size,
                usage,
                mapped_at_creation: false,
            });
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            queue.write_buffer(&buffer, 0, &bytes);
            input_buffers.push(buffer);
        }

        let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;
        let layout = pipeline.get_bind_group_layout(0);
        let mut entries = Vec::new();
        for (i, buffer) in input_buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buffer.as_entire_binding(),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: loaded.bundle.input_arity,
            resource: output_buffer.as_entire_binding(),
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("avm-gpu bind group"),
            layout: &layout,
            entries: &entries,
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("avm-gpu dispatch"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(plan.groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging, 0, size);

        let submit_start = Instant::now();
        let submission = queue.submit(Some(encoder.finish()));
        let submit_ms = submit_start.elapsed().as_secs_f64() * 1000.0;

        let readback_start = Instant::now();
        let (sender, receiver) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });

        // The readback and the completion hook run inside the queue's
        // submitted-work-done callback, which wgpu invokes after the mapping
        // callbacks of the same submission. `done` carries the result back to
        // this thread; the poll below is what lets the callback run.
        let (done, completed) = mpsc::channel::<Result<Vec<u32>, String>>();
        let callback_staging = staging.clone();
        let callback_size = size;
        queue.on_submitted_work_done(move || {
            let mapped = match receiver.try_recv() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(format!("buffer mapping failed: {e}")),
                Err(_) => Err("map callback did not arrive before completion".to_string()),
            };
            let result = mapped.map(|()| {
                let words = {
                    let view = callback_staging.slice(..).get_mapped_range();
                    let mut bytes = vec![0u8; callback_size as usize];
                    bytes.copy_from_slice(&view);
                    drop(view);
                    bytes
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect::<Vec<u32>>()
                };
                if let Some(hook) = on_ready {
                    hook(&words);
                }
                callback_staging.unmap();
                words
            });
            let _ = done.send(result);
        });

        match device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        }) {
            Ok(_) => {}
            Err(wgpu::PollError::Timeout) => {
                return Err(RunFailure::Refused(
                    "callback/poll timed out after 30s; buffers retained, not reused".to_string(),
                ))
            }
            Err(e) => {
                return Err(RunFailure::Refused(format!(
                    "device poll failed: {e}; buffers retained, not reused"
                )))
            }
        }
        if let Some(message) = lost.0.lock().unwrap().take() {
            return Err(RunFailure::Refused(message));
        }
        let output = match completed.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(words)) => words,
            Ok(Err(message)) => return Err(RunFailure::Refused(message)),
            Err(e) => {
                return Err(RunFailure::Refused(format!(
                    "work-done callback did not arrive: {e}"
                )))
            }
        };
        let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
        Ok(Timing {
            output,
            dispatch_count: plan.groups,
            prepare_ms,
            upload_ms,
            submit_ms,
            readback_ms,
        })
    }

}

#[cfg(feature = "device")]
pub use device_path::{run, run_with_completion, DeviceOutput, RunFailure};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        Evidence, KernelIr, MapKind, MemoryPlan, Stmt,
    };

    fn bundle(arity: u32) -> KernelBundle {
        KernelBundle {
            schema: crate::schema::SCHEMA.to_string(),
            backend: "wgsl".to_string(),
            numeric_profile: crate::schema::NUMERIC_PROFILE.to_string(),
            entry: "M:f".to_string(),
            input_arity: arity,
            compiler: "test".to_string(),
            sources: Vec::new(),
            kernel_ir: KernelIr {
                entry: "M:f".to_string(),
                input_arity: arity,
                helpers: Vec::new(),
                body: Stmt::Return {
                    value: crate::schema::Expr::Param { index: 0 },
                },
                memory: MemoryPlan {
                    map: if arity == 1 { MapKind::Map1 } else { MapKind::Map2 },
                    output_index: "identity".to_string(),
                    guard_before_load: true,
                    workgroup_size: WORKGROUP_SIZE,
                },
            },
            ir_sha256: String::new(),
            wgsl_sha256: String::new(),
            requirements: serde_json::json!({ "bindings": [] }),
            evidence: Evidence {
                source: "test".to_string(),
                shader_validation: "test".to_string(),
                device_kind: "test".to_string(),
                execution: "test".to_string(),
                proof: "test".to_string(),
            },
            proof_manifest: None,
        }
    }

    fn limits() -> LimitsView {
        LimitsView {
            max_buffer_size: 1 << 30,
            max_storage_buffer_binding_size: 1 << 28,
            max_compute_workgroups_per_dimension: 65535,
            max_compute_workgroup_size_x: 256,
        }
    }

    #[test]
    fn preflight_computes_groups_and_bytes() {
        let p = preflight(&bundle(2), &[vec![0; 64], vec![0; 64]], &limits()).unwrap();
        assert_eq!(p.groups, 1);
        assert_eq!(p.bytes, 256);
        let p = preflight(&bundle(2), &[vec![0; 65], vec![0; 65]], &limits()).unwrap();
        assert_eq!(p.groups, 2);
    }

    #[test]
    fn preflight_zero_is_empty_and_undispatched() {
        let p = preflight(&bundle(1), &[Vec::new()], &limits()).unwrap();
        assert_eq!(p.groups, 0);
        assert_eq!(p.bytes, 0);
    }

    #[test]
    fn preflight_refuses_bad_shapes() {
        assert!(preflight(&bundle(2), &[vec![0; 4]], &limits()).is_err());
        assert!(preflight(&bundle(2), &[vec![0; 4], vec![0; 5]], &limits()).is_err());
        assert!(preflight(&bundle(1), &[], &limits()).is_err());
    }

    #[test]
    fn preflight_refuses_limits_before_buffers() {
        let mut small = limits();
        small.max_buffer_size = 100;
        assert!(preflight(&bundle(1), &[vec![0; 26]], &small).is_err());
        let mut binding = limits();
        binding.max_storage_buffer_binding_size = 100;
        assert!(preflight(&bundle(1), &[vec![0; 26]], &binding).is_err());
        let mut dispatch = limits();
        dispatch.max_compute_workgroups_per_dimension = 1;
        assert!(preflight(&bundle(1), &[vec![0; 65]], &dispatch).is_err());
        let mut workgroup = limits();
        workgroup.max_compute_workgroup_size_x = 32;
        assert!(preflight(&bundle(1), &[vec![0; 1]], &workgroup).is_err());
    }
}
