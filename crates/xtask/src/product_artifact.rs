//! Prepare an immutable product artifact (Lom, Provlita, Hagia) from one
//! signed revision, for the attended tty4 gates.
//!
//! The same custody rules as `prepare-bemenu-artifact`: SOURCE AUTHORIZATION
//! (`git verify-commit`, status G) happens only here, the build input is
//! `git archive` of the signed commit whose extracted tree must hash to
//! exactly that commit's tree, the build runs low-priority with two jobs in a
//! private process group with a deadline and a log cap, and the output
//! directory is created last and made read-only. ARTIFACT BINDING is the
//! gates' separate job: the manifest is not signed, so they require the
//! operator's expected commit and digests (printed here) for every file. Nothing is read from the
//! source checkout's working tree and no sibling checkout is consulted: Rust
//! products build `--offline --locked` from their own pinned lock file, so
//! their dependencies must already be fetched (`cargo fetch --locked` in the
//! product repository).
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bemenu_artifact::{SignedTree, inputs, set_mode, signed_tree, tail, wait_logged};
use crate::{read, sha256};

const USAGE: &str = "usage: cargo xtask prepare-product-artifact <lom|provlita|hagia> \
                     <source-repo> <signed-commit> <new-output-dir>";
pub const MANIFEST: &str = "product-artifact.manifest";
pub const COMMIT_OBJECT: &str = "source.commit";
pub const CONFIG: &str = "config.kdl";
const CARGO_TIMEOUT: Duration = Duration::from_secs(3600);
const NIM_TIMEOUT: Duration = Duration::from_secs(900);

/// One product's build recipe.
pub struct Product {
    pub name: &'static str,
    /// The executable's file name, in the output directory too.
    pub binary: &'static str,
    /// A configuration shipped in the signed tree, copied as `config.kdl`.
    pub config: Option<&'static str>,
    kind: Kind,
}

enum Kind {
    Cargo,
    Nim { main: &'static str },
}

pub const PRODUCTS: [Product; 3] = [
    Product {
        name: "lom",
        binary: "lom",
        config: Some("examples/minimal/live-shell.kdl"),
        kind: Kind::Cargo,
    },
    Product {
        name: "provlita",
        binary: "provlita",
        config: Some("examples/minimal/config.kdl"),
        kind: Kind::Cargo,
    },
    Product {
        name: "hagia",
        binary: "hagia",
        config: None,
        kind: Kind::Nim {
            main: "src/hagia.nim",
        },
    },
];

pub fn product(name: &str) -> Result<&'static Product, String> {
    PRODUCTS
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("unknown product {name:?}; {USAGE}"))
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let [name, source, commit, output] = args else {
        return Err(USAGE.into());
    };
    let product = product(name)?;
    let (source, output) = inputs(source, commit, output)?;
    let SignedTree {
        scratch,
        _scratch,
        tree_dir,
        raw,
        tree,
        signer,
    } = signed_tree(&source, commit, product.name)?;

    let config = match product.config {
        Some(path) => {
            let config = tree_dir.join(path);
            if !std::fs::symlink_metadata(&config).is_ok_and(|m| m.is_file()) {
                return Err(format!("{} {commit} has no regular {path}", product.name));
            }
            Some(read(&config)?)
        }
        None => None,
    };

    // Low-priority, two-job build inside the scratch tree only.
    let log = scratch.join("build.log");
    let log_file = File::create(&log).map_err(|e| e.to_string())?;
    let out = scratch.join("out");
    std::fs::create_dir(&out).map_err(|e| e.to_string())?;
    let (mut command, binary, limit) = match product.kind {
        Kind::Cargo => {
            let mut command = Command::new("nice");
            command
                .args(["-n", "19", "cargo", "build", "--offline", "--locked"])
                .args(["--release", "--jobs", "2"])
                .env("CARGO_TARGET_DIR", &out)
                .env("CARGO_BUILD_JOBS", "2")
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env_remove("CARGO_BUILD_TARGET");
            let binary = out.join("release").join(product.binary);
            (command, binary, CARGO_TIMEOUT)
        }
        Kind::Nim { main } => {
            let binary = out.join(product.binary);
            let mut command = Command::new("nice");
            command
                .args(["-n", "19", "nim", "c", "-d:release", "--hints:off"])
                .args(["--path:src", "--parallelBuild:2"])
                .arg(format!("--nimcache:{}", scratch.join("nimcache").display()))
                .arg(format!("-o:{}", binary.display()))
                .arg(main);
            (command, binary, NIM_TIMEOUT)
        }
    };
    let child = command
        .current_dir(&tree_dir)
        .process_group(0)
        .env("GIT_DIR", scratch.join("no-git"))
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file)
        .spawn()
        .map_err(|e| format!("build {}: {e}", product.name))?;
    let what = format!("build {}", product.name);
    if let Err(error) = wait_logged(child, &log, limit, &what) {
        return Err(format!("{error}\n{}", tail(&log)));
    }
    if !std::fs::symlink_metadata(&binary).is_ok_and(|m| m.is_file()) {
        return Err(format!("build produced no regular {}", product.binary));
    }

    // Immutable output: created last, removed again if any step fails.
    std::fs::create_dir(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let written = write_output(product, &output, &binary, &raw, config.as_deref())
        .map(|(binary_sha256, config_sha256)| {
            [
                "schema=1".to_owned(),
                format!("product={}", product.name),
                format!("binary={}", product.binary),
                format!("binary_sha256={binary_sha256}"),
                format!("source_commit={commit}"),
                format!("source_tree={tree}"),
                "signature_status=G".to_owned(),
                format!("signer_fingerprint={signer}"),
                format!("config={}", if config.is_some() { CONFIG } else { "none" }),
                format!("config_sha256={config_sha256}"),
            ]
            .join("\n")
                + "\n"
        })
        .and_then(|manifest| {
            std::fs::write(output.join(MANIFEST), &manifest).map_err(|e| e.to_string())?;
            set_mode(&output.join(MANIFEST), 0o444)?;
            set_mode(&output, 0o555)?;
            Ok(manifest)
        });
    match written {
        Ok(manifest) => {
            // The operator passes these digests back to the gates, which bind
            // the files to them; the manifest itself is not signed.
            let field = |key: &str| {
                manifest
                    .lines()
                    .find_map(|l| l.strip_prefix(key))
                    .unwrap_or_default()
                    .to_owned()
            };
            Ok(vec![format!(
                "product_artifact status=prepared product={} commit={commit} binary_sha256={} config_sha256={} signer={signer} dir={}",
                product.name,
                field("binary_sha256="),
                field("config_sha256="),
                output.display()
            )])
        }
        Err(error) => {
            let _ = set_mode(&output, 0o700);
            let _ = std::fs::remove_dir_all(&output);
            Err(error)
        }
    }
}

/// Copy the binary, config and raw commit; returns (binary, config) digests.
fn write_output(
    product: &Product,
    output: &Path,
    binary: &Path,
    raw: &[u8],
    config: Option<&[u8]>,
) -> Result<(String, String), String> {
    let copy = output.join(product.binary);
    std::fs::copy(binary, &copy).map_err(|e| format!("copy {}: {e}", product.binary))?;
    let digest = sha256(&read(&copy)?);
    std::fs::write(output.join(COMMIT_OBJECT), raw).map_err(|e| e.to_string())?;
    set_mode(&copy, 0o555)?;
    set_mode(&output.join(COMMIT_OBJECT), 0o444)?;
    let config_digest = match config {
        Some(bytes) => {
            std::fs::write(output.join(CONFIG), bytes).map_err(|e| e.to_string())?;
            set_mode(&output.join(CONFIG), 0o444)?;
            sha256(bytes)
        }
        None => "none".to_owned(),
    };
    Ok((digest, config_digest))
}
