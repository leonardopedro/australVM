//! `vm.create` payload for cloud-hypervisor — the exact documented shape
//! from `../cloud-hypervisor-build/cloud-hypervisor/docs/api.md` ("Create a
//! Virtual Machine"): `cpus`, `payload`, `disks`, `rng`, `net`. Fields this
//! subsystem does not yet drive (VFIO devices, `platform`, `iommu`) are
//! intentionally absent rather than invented.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cpus {
    pub boot_vcpus: u16,
    pub max_vcpus: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Payload {
    /// Path to the guest kernel image. cloud-hypervisor takes both a
    /// `vmlinux` ELF and a compressed bzImage (verified 2026-10-18: the
    /// task-7.3 run boots Linux 6.12's `bzImage` through this payload).
    pub kernel: String,
    pub cmdline: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disk {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rng {
    pub src: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Net {
    pub ip: String,
    pub mask: String,
    pub mac: String,
}

/// A cloud-hypervisor console sink (`vmm/src/vm_config.rs`
/// `ConsoleConfig`: `mode` + optional `file`). The `serial` device —
/// the guest's `ttyS0` — defaults to mode `Null` (output dropped), so
/// the §5 in-guest timing run selects `File` and reads the guest
/// kernel's own timestamps back from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsoleSink {
    /// cloud-hypervisor `ConsoleOutputMode` name (`"File"`, `"Tty"`, …).
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VmConfig {
    pub cpus: Cpus,
    pub payload: Payload,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disks: Vec<Disk>,
    pub rng: Rng,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net: Vec<Net>,
    /// Where the legacy serial port (`console=ttyS0`) goes. Absent =
    /// cloud-hypervisor's default (`Null`); the documented shape in
    /// `docs/api.md`'s example omits it, and so does every payload
    /// unless a run opts in via [`VmConfig::with_serial_file`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<ConsoleSink>,
}

/// Which driver-domain shape a launch targets
/// (docs/spectrum_plan.md §8). `LinuxMicroVm` is the §1 bare-metal profile
/// (only Linux carries VFIO pass-through and vendor firmware);
/// `HermitUnikernel` is the cloud profile — one ELF (app + kernel linked
/// together, `../hermit-rs/hermit/README.md`), virtio-only devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverDomainProfile {
    LinuxMicroVm,
    HermitUnikernel,
}

impl DriverDomainProfile {
    /// vCPU policy. The bare-metal Linux domain keeps its 2 (the
    /// [`VmConfig::driver_domain`] shape); the cloud profile pins 1 —
    /// `../hermit-rs/k8s/httpd.yml` deploys `cpu: cores: 1`.
    pub fn default_vcpus(self) -> u16 {
        match self {
            DriverDomainProfile::LinuxMicroVm => 2,
            DriverDomainProfile::HermitUnikernel => 1,
        }
    }

    /// Guest memory budget in MiB. Unmodelled for the Linux micro-VM (this
    /// subsystem does not drive cloud-hypervisor's `memory` field and does
    /// not invent it); 64 for the Hermit profile — the `k8s/httpd.yml`
    /// `requests: memory: 64M` shape.
    pub fn default_memory_mib(self) -> Option<u32> {
        match self {
            DriverDomainProfile::LinuxMicroVm => None,
            DriverDomainProfile::HermitUnikernel => Some(64),
        }
    }
}

/// The cloud-profile (Hermit) domain descriptor. This is deliberately **not**
/// a cloud-hypervisor `vm.create` payload: the Hermit runners (Uhyve, or
/// QEMU + the Hermit loader — `../hermit-rs/hermit/README.md` "Running")
/// take a unikernel image plus CLI resources, so the domain shape is its
/// own record rather than a pretend REST body. Keys are only what the
/// runner actually consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HermitConfig {
    pub cpus: Cpus,
    pub memory_mib: u32,
    /// Path to the unikernel ELF (app + kernel linked together).
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net: Vec<Net>,
}

impl HermitConfig {
    /// The `../hermit-rs/k8s/httpd.yml` shape: 1 core, 64 MiB, virtio-only.
    pub fn new(image: impl Into<String>) -> Self {
        let profile = DriverDomainProfile::HermitUnikernel;
        HermitConfig {
            cpus: Cpus {
                boot_vcpus: profile.default_vcpus(),
                max_vcpus: profile.default_vcpus(),
            },
            memory_mib: profile.default_memory_mib().expect("hermit pins memory"),
            image: image.into(),
            net: Vec::new(),
        }
    }

    pub fn with_net(
        mut self,
        ip: impl Into<String>,
        mask: impl Into<String>,
        mac: impl Into<String>,
    ) -> Self {
        self.net.push(Net {
            ip: ip.into(),
            mask: mask.into(),
            mac: mac.into(),
        });
        self
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.cpus.max_vcpus < self.cpus.boot_vcpus {
            return Err(ConfigError::MaxVcpusBelowBoot);
        }
        if self.cpus.boot_vcpus == 0 {
            return Err(ConfigError::ZeroVcpus);
        }
        if self.image.is_empty() {
            return Err(ConfigError::EmptyImage);
        }
        for n in &self.net {
            if !valid_mac(&n.mac) {
                return Err(ConfigError::BadMac(n.mac.clone()));
            }
            if valid_ipv4(&n.ip) != Some(true) || valid_ipv4(&n.mask) != Some(true) {
                return Err(ConfigError::BadIp(
                    "ip/mask",
                    format!("{}/{}", n.ip, n.mask),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("HermitConfig serializes")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    MaxVcpusBelowBoot,
    ZeroVcpus,
    EmptyKernel,
    EmptyCmdline,
    EmptyImage,
    BadMac(String),
    BadIp(&'static str, String),
}

impl VmConfig {
    /// Minimal driver-domain config: N vcpus, kernel + cmdline, urandom.
    pub fn driver_domain(kernel: impl Into<String>, cmdline: impl Into<String>) -> Self {
        VmConfig {
            cpus: Cpus {
                boot_vcpus: DriverDomainProfile::LinuxMicroVm.default_vcpus(),
                max_vcpus: DriverDomainProfile::LinuxMicroVm.default_vcpus(),
            },
            payload: Payload {
                kernel: kernel.into(),
                cmdline: cmdline.into(),
            },
            disks: Vec::new(),
            rng: Rng {
                src: "/dev/urandom".to_string(),
            },
            net: Vec::new(),
            serial: None,
        }
    }

    /// Send the guest's `ttyS0` console to a file (cloud-hypervisor's
    /// `serial` device, mode `File`). The in-guest VirtIO-ready timing
    /// (§5) reads the guest kernel's `printk.time` stamps back from it.
    pub fn with_serial_file(mut self, path: impl Into<String>) -> Self {
        self.serial = Some(ConsoleSink {
            mode: "File".to_string(),
            file: Some(path.into()),
        });
        self
    }

    pub fn with_disk(mut self, path: impl Into<String>) -> Self {
        self.disks.push(Disk { path: path.into() });
        self
    }

    pub fn with_net(
        mut self,
        ip: impl Into<String>,
        mask: impl Into<String>,
        mac: impl Into<String>,
    ) -> Self {
        self.net.push(Net {
            ip: ip.into(),
            mask: mask.into(),
            mac: mac.into(),
        });
        self
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.cpus.max_vcpus < self.cpus.boot_vcpus {
            return Err(ConfigError::MaxVcpusBelowBoot);
        }
        if self.payload.kernel.is_empty() {
            return Err(ConfigError::EmptyKernel);
        }
        if self.payload.cmdline.is_empty() {
            return Err(ConfigError::EmptyCmdline);
        }
        for n in &self.net {
            if !valid_mac(&n.mac) {
                return Err(ConfigError::BadMac(n.mac.clone()));
            }
            if valid_ipv4(&n.ip) != Some(true) || valid_ipv4(&n.mask) != Some(true) {
                return Err(ConfigError::BadIp(
                    "ip/mask",
                    format!("{}/{}", n.ip, n.mask),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("VmConfig serializes")
    }
}

/// XX:XX:XX:XX:XX:XX, hex digits only.
fn valid_mac(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `Some(true)` iff dotted-quad with octets 0..255; `Some(false)` otherwise.
fn valid_ipv4(s: &str) -> Option<bool> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return Some(false);
    }
    for p in &parts {
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return Some(false);
        }
        match p.parse::<u16>() {
            Ok(v) if v <= 255 => {}
            _ => return Some(false),
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_api_md_example_shape() {
        // The exact example payload of docs/api.md, modulo paths.
        let cfg = VmConfig {
            cpus: Cpus {
                boot_vcpus: 4,
                max_vcpus: 4,
            },
            payload: Payload {
                kernel: "/opt/clh/kernel/vmlinux-virtio-fs-virtio-iommu".to_string(),
                cmdline: "console=ttyS0 console=hvc0 root=/dev/vda1 rw".to_string(),
            },
            disks: vec![Disk {
                path: "/opt/clh/images/focal-server-cloudimg-amd64.raw".to_string(),
            }],
            rng: Rng {
                src: "/dev/urandom".to_string(),
            },
            net: vec![Net {
                ip: "192.168.10.10".to_string(),
                mask: "255.255.255.0".to_string(),
                mac: "12:34:56:78:90:01".to_string(),
            }],
            // The api.md example carries no `serial` key; None keeps it
            // out of the serialized payload.
            serial: None,
        };
        cfg.validate().unwrap();
        let v: serde_json::Value = serde_json::from_str(&cfg.to_json()).unwrap();
        assert_eq!(v["cpus"]["boot_vcpus"], 4);
        assert_eq!(
            v["payload"]["kernel"],
            "/opt/clh/kernel/vmlinux-virtio-fs-virtio-iommu"
        );
        assert_eq!(
            v["disks"][0]["path"],
            "/opt/clh/images/focal-server-cloudimg-amd64.raw"
        );
        assert_eq!(v["rng"]["src"], "/dev/urandom");
        assert_eq!(v["net"][0]["mac"], "12:34:56:78:90:01");
        // Every documented key present, nothing invented at top level.
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["cpus", "disks", "net", "payload", "rng"]);
    }

    #[test]
    fn serial_sink_is_opt_in_and_matches_the_console_config_shape() {
        // The §5 in-guest timing run: cloud-hypervisor's serial device
        // (`vmm/src/vm_config.rs` ConsoleConfig { file, mode }) as an
        // opt-in `File` sink — and only then does the key appear.
        let cfg = VmConfig::driver_domain("/run/avm/kernel", "console=ttyS0")
            .with_serial_file("/run/avm/console/serial.log");
        cfg.validate().unwrap();
        let v: serde_json::Value = serde_json::from_str(&cfg.to_json()).unwrap();
        assert_eq!(v["serial"]["mode"], "File");
        assert_eq!(v["serial"]["file"], "/run/avm/console/serial.log");
        // Without the opt-in the key is absent (CH default = Null).
        let plain = VmConfig::driver_domain("/run/avm/kernel", "console=ttyS0");
        let v: serde_json::Value = serde_json::from_str(&plain.to_json()).unwrap();
        assert!(
            v.get("serial").is_none(),
            "serial stays out of the payload unless a run opts in"
        );
    }

    #[test]
    fn driver_domain_config_is_valid_and_minimal() {
        let cfg = VmConfig::driver_domain("/run/avm/kernel", "console=hvc0");
        cfg.validate().unwrap();
        let v: serde_json::Value = serde_json::from_str(&cfg.to_json()).unwrap();
        assert!(
            v.get("disks").is_none(),
            "empty optional lists stay out of the payload"
        );
        assert!(v.get("net").is_none());
    }

    #[test]
    fn hermit_domain_matches_the_k8s_httpd_shape() {
        let cfg = HermitConfig::new("/run/avm/hermit-domain");
        cfg.validate().unwrap();
        // ../hermit-rs/k8s/httpd.yml: `cpu: cores: 1`, `memory: 64M`.
        assert_eq!(
            cfg.cpus,
            Cpus {
                boot_vcpus: 1,
                max_vcpus: 1
            }
        );
        assert_eq!(cfg.memory_mib, 64);
        let v: serde_json::Value = serde_json::from_str(&cfg.to_json()).unwrap();
        assert!(
            v.get("disks").is_none(),
            "virtio-only cloud profile: no disks"
        );
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        // No disks, and an empty virtio net list stays out of the shape —
        // interfaces arrive via `with_net` (the k8s manifest's bridge NIC).
        assert_eq!(keys, vec!["cpus", "image", "memory_mib"]);
        let with_net = HermitConfig::new("/run/avm/hermit-domain").with_net(
            "10.0.0.2",
            "255.255.255.0",
            "aa:bb:cc:dd:ee:ff",
        );
        with_net.validate().unwrap();
        let v: serde_json::Value = serde_json::from_str(&with_net.to_json()).unwrap();
        assert_eq!(v["net"][0]["ip"], "10.0.0.2");
    }

    #[test]
    fn hermit_config_validates_image_memory_and_net() {
        assert_eq!(
            HermitConfig::new("").validate(),
            Err(ConfigError::EmptyImage)
        );
        let mut cfg = HermitConfig::new("/img");
        cfg.cpus = Cpus {
            boot_vcpus: 0,
            max_vcpus: 0,
        };
        assert_eq!(cfg.validate(), Err(ConfigError::ZeroVcpus));

        let cfg = HermitConfig::new("/img").with_net("10.0.0.1", "255.255.255.0", "not-a-mac");
        assert!(matches!(cfg.validate(), Err(ConfigError::BadMac(_))));

        let cfg = HermitConfig::new("/img").with_net("300.1.1.1", "255.0.0.0", "aa:bb:cc:dd:ee:ff");
        assert!(matches!(cfg.validate(), Err(ConfigError::BadIp(..))));
    }

    #[test]
    fn profile_policy_is_pinned_to_the_documented_shapes() {
        // Bare-metal profile: 2 vcpus (driver_domain), no modelled memory.
        assert_eq!(DriverDomainProfile::LinuxMicroVm.default_vcpus(), 2);
        assert_eq!(DriverDomainProfile::LinuxMicroVm.default_memory_mib(), None);
        // Cloud profile: the k8s/httpd.yml shape.
        assert_eq!(DriverDomainProfile::HermitUnikernel.default_vcpus(), 1);
        assert_eq!(
            DriverDomainProfile::HermitUnikernel.default_memory_mib(),
            Some(64)
        );
    }

    #[test]
    fn invalid_configs_rejected() {
        let mut cfg = VmConfig::driver_domain("/k", "cmd");
        cfg.cpus = Cpus {
            boot_vcpus: 4,
            max_vcpus: 2,
        };
        assert_eq!(cfg.validate(), Err(ConfigError::MaxVcpusBelowBoot));

        let cfg = VmConfig::driver_domain("", "cmd");
        assert_eq!(cfg.validate(), Err(ConfigError::EmptyKernel));
        let cfg = VmConfig::driver_domain("/k", "");
        assert_eq!(cfg.validate(), Err(ConfigError::EmptyCmdline));

        let cfg =
            VmConfig::driver_domain("/k", "cmd").with_net("10.0.0.1", "255.255.255.0", "not-a-mac");
        assert!(matches!(cfg.validate(), Err(ConfigError::BadMac(_))));

        let cfg = VmConfig::driver_domain("/k", "cmd").with_net(
            "300.1.1.1",
            "255.0.0.0",
            "aa:bb:cc:dd:ee:ff",
        );
        assert!(matches!(cfg.validate(), Err(ConfigError::BadIp(..))));
    }
}
