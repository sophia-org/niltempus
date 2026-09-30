//! Prepare an immutable product artifact (Lom, Provlita, Hagia) from one
//! signed revision, for the attended tty4 gates. `build` here is the one
//! corrected builder that `prepare-wm-pair` and `prepare-physical-inputs`
//! use too.
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
//!
//! Every build writes only below the caller's private `--build-dir` (a new,
//! randomly named scratch there; no shared or predictable /tmp path). Nim
//! products build only from a REVIEWED dependency manifest whose sha256 is
//! supplied separately (crate::nim_deps): the closure is staged read-only and
//! verified before and after, the host toolchain must be the reviewed one
//! before and after, and the compiler is a verified, read-only STAGED copy
//! of the reviewed installation (binary, installation configuration and
//! stdlib; crate::nim_install) whose configuration is traced and refused if
//! it reaches anything ambient. It runs in bwrap with no network and with
//! /home, /opt, /root and the live Nim installation hidden, skipping nimble
//! paths and the user, parent and project configurations. The effective nim
//! command and the configuration identity are recorded in the artifact.
//! After every build the staged source tree must still hash to the signed
//! commit's tree.
use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bemenu_artifact::{SignedTree, inputs, set_mode, signed_tree_under, tail, wait_logged};
use crate::nim_deps::{Reviewed, Toolchain, load_reviewed};
use crate::records::encode_value;
use crate::{read, sha256};

const USAGE: &str = "usage: cargo xtask prepare-product-artifact <lom|provlita|hagia> \
                     <source-repo> <signed-commit> <new-output-dir> --build-dir=/ABS \
                     [--nim-deps=/ABS --nim-deps-sha256=<64 hex>] (Nim products only, required)";
pub const MANIFEST: &str = "product-artifact.manifest";
pub const COMMIT_OBJECT: &str = "source.commit";
pub const CONFIG: &str = "config.kdl";
/// The reviewed dependency manifest a Nim product was built from.
pub const NIM_DEPS: &str = "nim-deps.manifest";
/// Recorded in every Nim artifact manifest.
pub const TOOLCHAIN_NOTE: &str = "recorded-not-a-reproducible-closure";
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

pub const PRODUCTS: [Product; 4] = [
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
    // Hagia's shell partner; packaged with it as the WM pair
    // (`prepare-wm-pair`), never selectable for the tty4 gates on its own.
    Product {
        name: "narthex",
        binary: "narthex",
        config: None,
        kind: Kind::Nim {
            main: "src/narthex.nim",
        },
    },
];

pub fn product(name: &str) -> Result<&'static Product, String> {
    PRODUCTS
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("unknown product {name:?}; {USAGE}"))
}

/// `--key=value` options, each allowed key at most once, none empty.
pub(crate) fn options<'a>(
    args: &'a [String],
    allowed: &[&str],
    usage: &str,
) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut options = BTreeMap::new();
    for arg in args {
        let (key, value) = arg
            .strip_prefix("--")
            .and_then(|a| a.split_once('='))
            .ok_or_else(|| format!("unexpected argument {arg:?}; {usage}"))?;
        if !allowed.contains(&key) || value.is_empty() {
            return Err(format!("unknown or empty option --{key}; {usage}"));
        }
        if options.insert(key, value).is_some() {
            return Err(format!("--{key} is repeated"));
        }
    }
    Ok(options)
}

/// A private build directory: absolute, owned, 0700, not a link.
pub(crate) fn build_dir(value: Option<&&str>) -> Result<PathBuf, String> {
    let dir = value.ok_or("--build-dir is required (no default)")?;
    if !dir.starts_with('/') {
        return Err(format!("--build-dir must be absolute: {dir}"));
    }
    crate::package_desktop::private_dir(Path::new(dir))?;
    Ok(PathBuf::from(dir))
}

/// The reviewed dependency manifest and its independently supplied sha256.
#[derive(Clone, Copy)]
pub struct NimDeps<'a> {
    pub path: &'a Path,
    pub sha256: &'a str,
}

/// `--{prefix}nim-deps` and `--{prefix}nim-deps-sha256`, both or neither.
pub(crate) fn nim_deps_option<'a>(
    options: &BTreeMap<&str, &'a str>,
    prefix: &str,
) -> Result<Option<NimDeps<'a>>, String> {
    let path = options.get(format!("{prefix}nim-deps").as_str());
    let digest = options.get(format!("{prefix}nim-deps-sha256").as_str());
    match (path, digest) {
        (Some(path), Some(digest)) => Ok(Some(NimDeps {
            path: Path::new(*path),
            sha256: digest,
        })),
        (None, None) => Ok(None),
        _ => Err(format!(
            "--{prefix}nim-deps and --{prefix}nim-deps-sha256 go together"
        )),
    }
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    if args.len() < 4 {
        return Err(USAGE.into());
    }
    let (head, rest) = args.split_at(4);
    let [name, source, commit, output] = head else {
        return Err(USAGE.into());
    };
    let product = product(name)?;
    if product.name == "narthex" {
        return Err("narthex is packaged with Hagia: use prepare-wm-pair".into());
    }
    let (source, output) = inputs(source, commit, output)?;
    let options = options(rest, &["build-dir", "nim-deps", "nim-deps-sha256"], USAGE)?;
    let build_dir = build_dir(options.get("build-dir"))?;
    let deps = nim_deps_option(&options, "")?;
    let built = build(product, &source, commit, &build_dir, deps)?;
    let SignedTree {
        raw,
        tree,
        signer,
        tree_dir,
        ..
    } = &built.tree;
    let binary = &built.binary;

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

    // Immutable output: created last, removed again if any step fails.
    std::fs::create_dir(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let written = write_output(product, &output, binary, raw, config.as_deref())
        .and_then(|digests| {
            if let Some(nim) = &built.nim_deps {
                std::fs::write(output.join(NIM_DEPS), &nim.reviewed.text)
                    .map_err(|e| e.to_string())?;
                set_mode(&output.join(NIM_DEPS), 0o444)?;
            }
            Ok(digests)
        })
        .map(|(binary_sha256, config_sha256)| {
            let nim = built.nim_deps.as_ref();
            let (deps, note) = match nim {
                Some(nim) => (nim.reviewed.sha256.clone(), TOOLCHAIN_NOTE),
                None => ("none".to_owned(), "none"),
            };
            let field = |f: fn(&NimBuild) -> String| nim.map_or_else(|| "none".to_owned(), f);
            [
                "schema=2".to_owned(),
                format!("product={}", product.name),
                format!("binary={}", product.binary),
                format!("binary_sha256={binary_sha256}"),
                format!("source_commit={commit}"),
                format!("source_tree={tree}"),
                "signature_status=G".to_owned(),
                format!("signer_fingerprint={signer}"),
                format!("config={}", if config.is_some() { CONFIG } else { "none" }),
                format!("config_sha256={config_sha256}"),
                format!("nim_deps_sha256={deps}"),
                format!("toolchain_identity={note}"),
                format!(
                    "nim_config_sha256={}",
                    field(|n| n.config_inventory_sha256.clone())
                ),
                format!("nim_config_read={}", field(NimBuild::config_read)),
                format!(
                    "nim_stdlib_sha256={}",
                    field(|n| n.stdlib_inventory_sha256.clone())
                ),
                format!("nim_command={}", field(NimBuild::command_line)),
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

/// A product built from its signed tree; the scratch (and binary) live until
/// this value is dropped.
pub struct Built {
    pub(crate) tree: SignedTree,
    pub binary: std::path::PathBuf,
    /// How a Nim product was built (None for Cargo products).
    pub nim_deps: Option<NimBuild>,
}

/// A Nim product's bound build: the reviewed closure, the effective
/// compiler command and the staged installation configuration's identity.
#[derive(Debug, Clone)]
pub struct NimBuild {
    pub reviewed: Reviewed,
    /// The effective nim argv, the staged compiler first.
    pub command: Vec<String>,
    pub config_inventory_sha256: String,
    pub stdlib_inventory_sha256: String,
    /// Every configuration file the compiler reads (relative to the staged
    /// prefix) with its sha256.
    pub config_files: Vec<(String, String)>,
}

impl NimBuild {
    /// The argv, each argument in the records encoding, space-joined: one
    /// unambiguous line.
    pub fn command_line(&self) -> String {
        self.command
            .iter()
            .map(|a| encode_value(a).unwrap_or_else(|_| "\"<control>\"".into()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `path:sha256` for every configuration file read, space-joined.
    pub fn config_read(&self) -> String {
        self.config_files
            .iter()
            .map(|(path, sha)| format!("{path}:{sha}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Signer authorization and the exact signed tree, staged in a private
/// scratch under `build_dir`, then the product's low-priority, two-job,
/// bounded build inside that scratch, then the post-build tree proof. Nim
/// products need their reviewed dependency manifest (`deps`); Cargo
/// products refuse one.
pub(crate) fn build(
    product: &Product,
    source: &Path,
    commit: &str,
    build_dir: &Path,
    deps: Option<NimDeps>,
) -> Result<Built, String> {
    // Refuse a missing or misplaced dependency manifest before anything is
    // authorized or staged.
    let deps = match (&product.kind, deps) {
        (Kind::Cargo, None) => None,
        (Kind::Cargo, Some(_)) => {
            return Err(format!(
                "{} is a Cargo product: it takes no --nim-deps",
                product.name
            ));
        }
        (Kind::Nim { .. }, None) => {
            return Err(format!(
                "{} needs its reviewed dependency manifest: --nim-deps=/ABS --nim-deps-sha256=<64 hex>",
                product.name
            ));
        }
        (Kind::Nim { .. }, Some(deps)) => Some(deps),
    };
    let tree = signed_tree_under(build_dir, source, commit, product.name)?;
    let (scratch, tree_dir) = (tree.scratch.clone(), tree.tree_dir.clone());
    let reviewed = match deps {
        None => None,
        Some(deps) => Some(load_reviewed(
            deps.path,
            deps.sha256,
            product.name,
            commit,
            &tree.tree,
        )?),
    };
    let mut staged = None;
    let mut install = None;
    let mut record = None;
    // Low-priority build inside the scratch tree, capped at two jobs.
    let log = scratch.join("build.log");
    let log_file = File::create(&log).map_err(|e| e.to_string())?;
    let out = scratch.join("out");
    std::fs::create_dir(&out).map_err(|e| e.to_string())?;
    let (mut command, binary, limit) = match product.kind {
        Kind::Cargo => {
            let mut command = Command::new("nice");
            command
                .args(["-n", "19", "cargo", "build", "--offline", "--locked"])
                .args(["--release", "--jobs", build_jobs()])
                .env("CARGO_TARGET_DIR", &out)
                .env("CARGO_BUILD_JOBS", build_jobs())
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env_remove("CARGO_BUILD_TARGET");
            let binary = out.join("release").join(product.binary);
            (command, binary, CARGO_TIMEOUT)
        }
        Kind::Nim { main } => {
            let reviewed = reviewed
                .as_ref()
                .expect("Nim products have a reviewed manifest");
            let toolchain = reviewed.check_toolchain()?;
            let deps = reviewed.stage(&scratch.join("nim-deps"))?;
            let prefix = crate::nim_install::stage(&toolchain, &scratch.join("nim"))?;
            let binary = out.join(product.binary);
            let home = scratch.join("home");
            std::fs::create_dir(&home).map_err(|e| e.to_string())?;
            let (command, argv) = nim_command(
                &toolchain,
                &prefix,
                &NimPaths {
                    scratch: &scratch,
                    home: &home,
                    tree: &tree_dir,
                    deps: &deps.root,
                    dep_dirs: &deps.dirs,
                    binary: &binary,
                },
                main,
            )?;
            record = Some((
                argv,
                prefix.config_inventory_sha256.clone(),
                prefix.stdlib_inventory_sha256.clone(),
                prefix.trace.files.clone(),
            ));
            staged = Some(deps);
            install = Some(prefix);
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
    // Post-build proofs: the source tree is still the signed tree, and the
    // closure and toolchain are still the reviewed ones.
    if crate::git_tree::inventory(&tree_dir)?.tree != tree.tree {
        return Err(format!(
            "the {} build changed its staged source tree",
            product.name
        ));
    }
    if let (Some(reviewed), Some(deps)) = (&reviewed, &staged) {
        reviewed.verify(deps)?;
        reviewed.check_toolchain()?;
    }
    if let Some(prefix) = &install {
        prefix.verify()?;
    }
    let nim_deps = match (reviewed, record) {
        (Some(reviewed), Some((command, config, stdlib, files))) => Some(NimBuild {
            reviewed,
            command,
            config_inventory_sha256: config,
            stdlib_inventory_sha256: stdlib,
            config_files: files,
        }),
        _ => None,
    };
    Ok(Built {
        tree,
        binary,
        nim_deps,
    })
}

/// The private paths one Nim build uses.
pub struct NimPaths<'a> {
    pub scratch: &'a Path,
    pub home: &'a Path,
    pub tree: &'a Path,
    pub deps: &'a Path,
    pub dep_dirs: &'a [PathBuf],
    pub binary: &'a Path,
}

/// The bounded, network-less, sandboxed command for one Nim build from the
/// staged installation, and the effective nim argv it runs.
pub fn nim_command(
    toolchain: &Toolchain,
    prefix: &crate::nim_install::Staged,
    paths: &NimPaths,
    main: &str,
) -> Result<(Command, Vec<String>), String> {
    let gcc = toolchain.tool("gcc")?;
    let mut argv = vec![
        prefix.nim.display().to_string(),
        "c".into(),
        "-d:release".into(),
        "--hints:off".into(),
    ];
    argv.extend(nim_flags(&prefix.lib, &gcc, paths.dep_dirs));
    argv.extend([
        "--path:src".to_owned(),
        format!("--parallelBuild:{}", build_jobs()),
        format!("--nimcache:{}", paths.scratch.join("nimcache").display()),
        format!("-o:{}", paths.binary.display()),
        main.to_owned(),
    ]);
    let command = sandboxed(toolchain, &prefix.root, paths, &argv)?;
    Ok((command, argv))
}

/// Preserve the preparation ceiling while allowing an explicitly serialized
/// build. Apply the same caller limit to Rust and Nim's C compilation.
pub(crate) fn build_jobs() -> &'static str {
    if std::env::var("CARGO_BUILD_JOBS").as_deref() == Ok("1") {
        "1"
    } else {
        "2"
    }
}

/// `argv` at nice 19 in bwrap: no network, a private /tmp, every home,
/// /opt, /root and the LIVE Nim installation (the reviewed compiler's
/// prefix, and /etc/nim) hidden, and only the private scratch (read-write),
/// the staged dependencies and the staged installation (read-only) bound
/// back. Only the staged copies can be read.
pub fn sandboxed(
    toolchain: &Toolchain,
    prefix: &Path,
    paths: &NimPaths,
    argv: &[String],
) -> Result<Command, String> {
    let gcc = toolchain.tool("gcc")?;
    let live_prefix = toolchain
        .tool("nim")?
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or("the reviewed nim has no installation prefix")?;
    let mut command = Command::new("nice");
    command
        .env_clear()
        .env("PATH", gcc.parent().ok_or("gcc has no directory")?)
        .env("HOME", paths.home)
        .env("LC_ALL", "C")
        .args(["-n", "19"])
        .arg(toolchain.tool("bwrap")?)
        .args(["--unshare-net", "--die-with-parent", "--ro-bind", "/", "/"])
        .args(["--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
    let hidden = [
        Path::new("/home"),
        Path::new("/opt"),
        Path::new("/root"),
        Path::new("/etc/nim"),
        live_prefix.as_path(),
    ];
    for dir in hidden {
        if dir.is_dir() {
            command.arg("--tmpfs").arg(dir);
        }
    }
    command
        .arg("--bind")
        .args([paths.scratch, paths.scratch])
        .arg("--ro-bind")
        .args([paths.deps, paths.deps])
        .arg("--ro-bind")
        .args([prefix, prefix])
        .arg("--chdir")
        .arg(paths.tree)
        .arg("--")
        .args(argv);
    Ok(command)
}

/// The Nim flags that make the dependency set and configuration explicit:
/// no nimble path, no user, parent or project configuration, the staged
/// stdlib, the reviewed gcc, and each staged dependency directory.
pub fn nim_flags(lib: &Path, gcc: &Path, deps: &[PathBuf]) -> Vec<String> {
    let mut flags = [
        "--noNimblePath",
        "--clearNimblePath",
        "--skipUserCfg:on",
        "--skipParentCfg:on",
        "--skipProjCfg:on",
    ]
    .map(str::to_owned)
    .to_vec();
    flags.push(format!("--lib:{}", lib.display()));
    flags.push("--cc:gcc".into());
    flags.push(format!("--gcc.exe:{}", gcc.display()));
    flags.push(format!("--gcc.linkerexe:{}", gcc.display()));
    for dir in deps {
        flags.push(format!("--path:{}", dir.display()));
    }
    flags
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
