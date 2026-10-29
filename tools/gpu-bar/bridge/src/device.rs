//! Bind DRM node identities to a physical PCI device before and after Vulkan setup.
use sophia_shell_gpu::ExternalGpuEvidence;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Device {
    pub(crate) path: PathBuf,
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) pci: String,
}

pub(crate) fn normalize_pci(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 12
        || bytes[4] != b':'
        || bytes[7] != b':'
        || bytes[10] != b'.'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10) || c.is_ascii_hexdigit())
        || u8::from_str_radix(&value[8..10], 16).map_or(true, |n| n > 31)
        || !matches!(bytes[11], b'0'..=b'7')
    {
        return Err("expected a PCI address domain:bus:slot.function".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn physical_device(sysfs: &Path, major: u32, minor: u32) -> Result<String, String> {
    let path = sysfs.join(format!("dev/char/{major}:{minor}/device"));
    let device = path
        .canonicalize()
        .map_err(|e| format!("DRM physical-device link: {e}"))?;
    normalize_pci(
        device
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("non-UTF8 physical device")?,
    )
}

fn require_physical(sysfs: &Path, major: u32, minor: u32, pci: &str) -> Result<(), String> {
    if physical_device(sysfs, major, minor)? != pci {
        return Err("DRM node resolves to a different physical GPU".into());
    }
    Ok(())
}

impl Device {
    pub(crate) fn resolve(path: &Path, pci: &str) -> Result<Self, String> {
        let pci = normalize_pci(pci)?;
        let node = path.to_str().ok_or("non-UTF8 render-node path")?;
        let minor: u32 = node
            .strip_prefix("/dev/dri/renderD")
            .ok_or("expected an absolute DRM render-node path")?
            .parse()
            .map_err(|_| "invalid render-node minor")?;
        if node != format!("/dev/dri/renderD{minor}") {
            return Err("render-node path is not canonical".into());
        }
        let metadata = path
            .symlink_metadata()
            .map_err(|e| format!("render-node metadata: {e}"))?;
        if !metadata.file_type().is_char_device() || rustix::fs::minor(metadata.rdev()) != minor {
            return Err("render node is not the named character device".into());
        }
        let major = rustix::fs::major(metadata.rdev());
        require_physical(Path::new("/sys"), major, minor, &pci)?;
        Ok(Self {
            path: path.to_owned(),
            major,
            minor,
            pci,
        })
    }

    pub(crate) fn verify(&self, evidence: &ExternalGpuEvidence) -> Result<(), String> {
        if evidence.render_node != self.path
            || evidence.device_major != self.major
            || evidence.device_minor != self.minor
        {
            return Err("renderer selected a different DRM identity".into());
        }
        require_physical(
            Path::new("/sys"),
            evidence.device_major,
            evidence.device_minor,
            &self.pci,
        )?;
        self.verify_adapter_pci(&evidence.adapter.device_pci_bus_id)
    }

    fn verify_adapter_pci(&self, pci: &str) -> Result<(), String> {
        if normalize_pci(pci)? != self.pci {
            return Err("Vulkan PCI identity disagrees with the DRM physical device".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/support/device.rs"]
mod tests;
