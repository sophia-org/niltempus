use super::*;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "gpu-device-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn device(&self, dev_t: &str, pci: &str) {
        let target = self.0.join("devices").join(pci);
        std::fs::create_dir_all(&target).unwrap();
        let node = self.0.join("dev/char").join(dev_t);
        std::fs::create_dir_all(&node).unwrap();
        symlink(&target, node.join("device")).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn physical_gpu_mapping_distinguishes_the_two_devices_not_the_node_kind() {
    let sysfs = Fixture::new();
    sysfs.device("226:0", "0000:03:00.0");
    sysfs.device("226:128", "0000:03:00.0");
    sysfs.device("226:129", "0000:16:00.0");
    assert_eq!(
        physical_device(&sysfs.0, 226, 0).unwrap(),
        physical_device(&sysfs.0, 226, 128).unwrap()
    );
    assert!(require_physical(&sysfs.0, 226, 128, "0000:03:00.0").is_ok());
    assert!(require_physical(&sysfs.0, 226, 129, "0000:03:00.0").is_err());
    assert!(require_physical(&sysfs.0, 226, 128, "0000:16:00.0").is_err());
    assert!(physical_device(&sysfs.0, 226, 130).is_err());
    let device = Device {
        path: "/dev/dri/renderD128".into(),
        major: 226,
        minor: 128,
        pci: "0000:03:00.0".into(),
    };
    assert!(device.verify_adapter_pci("").is_err());
    assert!(device.verify_adapter_pci("0000:03:00.0").is_ok());
    assert!(device.verify_adapter_pci("0000:16:00.0").is_err());
    assert!(device.verify_adapter_pci("malformed").is_err());
}

#[test]
fn pci_and_render_paths_are_strict_before_any_gpu_open() {
    assert_eq!(normalize_pci("ABCD:EF:1A.7").unwrap(), "abcd:ef:1a.7");
    for value in [
        "",
        "03:00.0",
        "0000:03:00.0\n",
        "0000:03:20.0",
        "0000:03:00.8",
        "0000:0z:00.0",
        "é000:03:00.0",
    ] {
        assert!(normalize_pci(value).is_err(), "{value}");
    }
    for path in [
        "/dev/null",
        "renderD128",
        "/dev/dri/card0",
        "/dev/dri/renderD0128",
        "/dev/dri/renderD128/../renderD128",
    ] {
        assert!(
            Device::resolve(Path::new(path), "0000:03:00.0").is_err(),
            "{path}"
        );
    }
}
