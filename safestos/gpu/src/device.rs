//! G09: real adapter enumeration and classification (`--features device`).
//!
//! This module asks wgpu for an adapter. It does not launch a kernel and it
//! does not infer hardware from `/dev/dri` nodes or `nvidia-smi`; the evidence
//! is the adapter wgpu actually returns.

use wgpu::{Backends, DeviceType, Instance, InstanceDescriptor, PowerPreference, RequestAdapterOptions};

#[derive(Debug, Clone)]
pub struct AdapterEvidence {
    pub backend: String,
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: String,
    pub physical: bool,
    pub max_workgroup_size_x: u32,
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroups_per_dimension: u32,
}

#[derive(Debug)]
pub enum ProbeError {
    /// No adapter was returned. Optional mode maps this to exit 77.
    Unavailable(String),
    /// An adapter exists but its type is not a physical GPU type.
    NotPhysical(AdapterEvidence),
    Failed(String),
}

fn classify(device_type: DeviceType) -> (String, bool) {
    match device_type {
        DeviceType::DiscreteGpu => ("discrete-gpu".to_string(), true),
        DeviceType::IntegratedGpu => ("integrated-gpu".to_string(), true),
        DeviceType::Cpu => ("cpu".to_string(), false),
        DeviceType::VirtualGpu => ("virtual-gpu".to_string(), false),
        DeviceType::Other => ("unknown".to_string(), false),
    }
}

/// An explicit, validated adapter choice. The empty selection is wgpu's own
/// default (`request_adapter`); a name or index makes the choice deliberate
/// instead of depending on backend enumeration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Case-insensitive substring of the adapter name.
    pub name: Option<String>,
    /// Position in `enumerate_adapters`, as printed by `probe --list`.
    pub index: Option<usize>,
}

impl Selection {
    pub fn is_default(&self) -> bool {
        self.name.is_none() && self.index.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = &self.name {
            if name.trim().is_empty() {
                return Err("adapter name selector is empty".to_string());
            }
        }
        Ok(())
    }
}

fn instance() -> Instance {
    let mut desc = InstanceDescriptor::new_without_display_handle();
    desc.backends = Backends::VULKAN | Backends::GL | Backends::METAL | Backends::DX12;
    Instance::new(desc)
}

fn evidence_for(adapter: &wgpu::Adapter) -> AdapterEvidence {
    let info = adapter.get_info();
    let limits = adapter.limits();
    let (device_type, physical) = classify(info.device_type);
    AdapterEvidence {
        backend: format!("{:?}", info.backend),
        name: info.name,
        vendor: info.vendor,
        device: info.device,
        device_type,
        physical,
        max_workgroup_size_x: limits.max_compute_workgroup_size_x,
        max_buffer_size: limits.max_buffer_size,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
    }
}

/// All adapters the instance can see, in the order `--adapter-index` refers to.
pub fn enumerate() -> Vec<AdapterEvidence> {
    let instance = instance();
    pollster::block_on(instance.enumerate_adapters(instance_backends()))
        .iter()
        .map(evidence_for)
        .collect()
}

fn instance_backends() -> Backends {
    Backends::VULKAN | Backends::GL | Backends::METAL | Backends::DX12
}

/// Enumerate and return an adapter plus its classified evidence. `probe` and
/// the runner both go through this, so they cannot disagree about the adapter.
pub(crate) fn request(
    selection: &Selection,
) -> Result<(wgpu::Adapter, AdapterEvidence), ProbeError> {
    selection.validate().map_err(ProbeError::Failed)?;
    let instance = instance();
    if selection.is_default() {
        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| ProbeError::Unavailable(format!("no adapter: {e}")))?;
        let evidence = evidence_for(&adapter);
        return Ok((adapter, evidence));
    }

    let adapters = pollster::block_on(instance.enumerate_adapters(instance_backends()));
    if adapters.is_empty() {
        return Err(ProbeError::Unavailable("no adapter: enumeration returned none".to_string()));
    }
    let all = adapters
        .iter()
        .map(evidence_for)
        .collect::<Vec<_>>();
    let chosen = match (&selection.name, selection.index) {
        (Some(name), index) => {
            let needle = name.to_ascii_lowercase();
            let matches = all
                .iter()
                .enumerate()
                .filter(|(_, e)| e.name.to_ascii_lowercase().contains(&needle))
                .collect::<Vec<_>>();
            let (i, evidence) = match matches.as_slice() {
                [(i, e)] => (*i, *e),
                [] => {
                    return Err(ProbeError::Failed(format!(
                        "no adapter matches {name:?}; available: {}",
                        describe(&all)
                    )))
                }
                many => {
                    return Err(ProbeError::Failed(format!(
                        "adapter selector {name:?} is ambiguous ({} matches: {}); use --adapter-index",
                        many.len(),
                        describe(&all)
                    )))
                }
            };
            if let Some(index) = index {
                if index != i {
                    return Err(ProbeError::Failed(format!(
                        "adapter selector {name:?} is adapter {i}, not index {index}"
                    )));
                }
            }
            evidence
        }
        (None, Some(index)) => match all.get(index) {
            Some(evidence) => evidence,
            None => {
                return Err(ProbeError::Failed(format!(
                    "adapter index {index} is out of range for {} adapter(s): {}",
                    all.len(),
                    describe(&all)
                )))
            }
        },
        (None, None) => unreachable!("the default selection returned above"),
    };
    let position = all
        .iter()
        .position(|e| e.name == chosen.name && e.backend == chosen.backend)
        .expect("enumerated evidence belongs to an enumerated adapter");
    Ok((adapters[position].clone(), chosen.clone()))
}

fn describe(all: &[AdapterEvidence]) -> String {
    all.iter()
        .enumerate()
        .map(|(i, e)| format!("{i}: {} ({}, {})", e.name, e.backend, e.device_type))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn probe(selection: &Selection) -> Result<AdapterEvidence, ProbeError> {
    let (_adapter, evidence) = request(selection)?;
    if evidence.physical {
        Ok(evidence)
    } else {
        Err(ProbeError::NotPhysical(evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gpu_types_count_as_physical() {
        assert_eq!(classify(DeviceType::DiscreteGpu), ("discrete-gpu".to_string(), true));
        assert_eq!(classify(DeviceType::IntegratedGpu), ("integrated-gpu".to_string(), true));
        assert_eq!(classify(DeviceType::Cpu), ("cpu".to_string(), false));
        assert_eq!(classify(DeviceType::VirtualGpu), ("virtual-gpu".to_string(), false));
        assert_eq!(classify(DeviceType::Other), ("unknown".to_string(), false));
    }
}

pub fn to_json(e: &AdapterEvidence) -> serde_json::Value {
    serde_json::json!({
        "backend": e.backend,
        "name": e.name,
        "vendor": e.vendor,
        "device": e.device,
        "device_type": e.device_type,
        "max_compute_workgroup_size_x": e.max_workgroup_size_x,
        "max_buffer_size": e.max_buffer_size,
        "max_storage_buffer_binding_size": e.max_storage_buffer_binding_size,
        "max_compute_workgroups_per_dimension": e.max_compute_workgroups_per_dimension,
    })
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn empty_name_is_refused_and_default_is_explicit() {
        assert!(Selection::default().is_default());
        assert!(Selection {
            name: Some(" ".to_string()),
            index: None
        }
        .validate()
        .is_err());
        assert!(Selection {
            name: Some("nvidia".to_string()),
            index: None
        }
        .validate()
        .is_ok());
        assert!(!Selection {
            name: None,
            index: Some(0)
        }
        .is_default());
    }

    #[test]
    fn describe_numbers_each_adapter() {
        let all = vec![
            evidence("Intel", "Vulkan"),
            evidence("NVIDIA GeForce GTX 1060", "Vulkan"),
        ];
        let text = describe(&all);
        assert!(text.starts_with("0: Intel (Vulkan"));
        assert!(text.contains("1: NVIDIA GeForce GTX 1060 (Vulkan"));
    }

    fn evidence(name: &str, backend: &str) -> AdapterEvidence {
        AdapterEvidence {
            backend: backend.to_string(),
            name: name.to_string(),
            vendor: 0,
            device: 0,
            device_type: "integrated-gpu".to_string(),
            physical: true,
            max_workgroup_size_x: 64,
            max_buffer_size: 1 << 20,
            max_storage_buffer_binding_size: 1 << 20,
            max_compute_workgroups_per_dimension: 1024,
        }
    }
}
