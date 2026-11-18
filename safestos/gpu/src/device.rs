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

pub fn probe() -> Result<AdapterEvidence, ProbeError> {
    let mut desc = InstanceDescriptor::new_without_display_handle();
    desc.backends = Backends::VULKAN | Backends::GL | Backends::METAL | Backends::DX12;
    let instance = Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|e| ProbeError::Unavailable(format!("no adapter: {e}")))?;
    let info = adapter.get_info();
    let limits = adapter.limits();
    let (device_type, physical) = classify(info.device_type);
    let evidence = AdapterEvidence {
        backend: format!("{:?}", info.backend),
        name: info.name,
        vendor: info.vendor,
        device: info.device,
        device_type,
        physical,
        max_workgroup_size_x: limits.max_compute_workgroup_size_x,
        max_buffer_size: limits.max_buffer_size,
    };
    if physical {
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
    })
}
