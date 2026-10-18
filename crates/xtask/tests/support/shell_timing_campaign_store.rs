//! Persistence-only controls: bytes here are not qualified native plans.
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "campaign-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn freeze_writes_exact_bytes_and_refuses_replacement() {
    let root = Root::new();
    let plans = vec![("first".into(), b"prepared plan".to_vec())];
    let dir = persist(&root.0, "campaign", b"manifest", "digest", plans.clone()).unwrap();
    assert_eq!(
        fs::read(dir.join("first/plan.json")).unwrap(),
        b"prepared plan"
    );
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), b"manifest");
    assert_eq!(fs::read(dir.join("READY")).unwrap(), b"digest");
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(dir.join("READY"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(persist(&root.0, "campaign", b"replacement", "other", plans).is_err());
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), b"manifest");
}

#[test]
fn partial_failure_is_retained_without_ready_and_cannot_be_reused() {
    let root = Root::new();
    // Inject a filesystem collision after the first arm has been persisted.
    // Public matrix validation prohibits duplicate arm ids before this layer.
    let plans = vec![
        ("first".into(), b"one".to_vec()),
        ("first".into(), b"two".to_vec()),
    ];
    assert!(persist(&root.0, "campaign", b"manifest", "digest", plans).is_err());
    let dir = root.0.join("campaign");
    assert!(!dir.join("READY").exists());
    assert_eq!(fs::read(dir.join("first/plan.json")).unwrap(), b"one");
    assert!(persist(&root.0, "campaign", b"retry", "digest", vec![]).is_err());
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), b"manifest");
}
