//! Artifact preparation and pin checks for Sophia desktop integration.
//!
//! This repository owns tests that combine Sophia with named desktop clients
//! (Sophia rule 13). It consumes Sophia's public crates at one pinned revision
//! and prepared client artifacts; it never builds against a client's checkout
//! or the C SDK's source tree.
pub mod bemenu_artifact;
pub mod c_sdk_pin;
pub mod direct_scanout_gate;
pub mod dock;
pub mod git_tree;
pub mod nim_deps;
pub mod nim_install;
pub mod package_desktop;
pub mod panel;
pub mod physical_inputs;
pub mod pins;
pub mod product_artifact;
pub mod records;
pub mod release_verify;
pub mod session;
pub mod session_preflight;
pub mod verify_archives;
pub mod wm_pair;

pub(crate) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn read(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
