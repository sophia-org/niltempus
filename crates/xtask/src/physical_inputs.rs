//! Prepare and verify the immutable inputs of a physical gate
//! (`cargo xtask prepare-physical-inputs`), so no runner builds in a source
//! tree.
//!
//! ```text
//! cargo xtask prepare-physical-inputs --sophia-root=/ABS --build-dir=/ABS \
//!     --out=/ABS/NEW --sophia-features=native-session|atomic-scanout-live \
//!     [--sophia-packages=sophia-cli|sophia-cli,sophia-wm-demo|sophia-cli,sophia-conformance] \
//!     [--hagia=/ABS/REPO --hagia-commit=<40> \
//!      --hagia-nim-deps=/ABS --hagia-nim-deps-sha256=<64>] \
//!     [--narthex=/ABS/REPO --narthex-commit=<40> \
//!      --narthex-nim-deps=/ABS --narthex-nim-deps-sha256=<64>] \
//!     [--profile=OWNER:RELATIVE-PATH ...]
//! cargo xtask prepare-physical-inputs verify --out=/ABS --manifest-sha256=<64>
//! ```
//!
//! Custody, in order: the full provisioning marker and CARGO_HOME; this
//! repository clean with a signed HEAD (bound); Sophia's clean checkout at
//! the pinned, signed revision; every source staged as its exact signed tree
//! (git archive, tree-hash proven) in a private scratch under `--build-dir`;
//! Sophia built there offline, `--locked`, at the caller's priority and
//! parallelism, bounded, into
//! `--build-dir/sophia-target`; Hagia and Narthex through the one corrected
//! builder (product_artifact::build) from their reviewed dependency
//! manifests; every staged tree re-proven after its build. `--profile`
//! names a file inside its OWNING staged source (`sophia:`, `hagia:`,
//! `narthex:` or `integration:`), never traversing or escaping it. Integration
//! profiles are read from an archive of the bound signed integration commit.
//!
//! The output is new, created last and made read-only: `bin/`, the exact
//! pinned Sophia tree (`sophia-tree/`, the runners' SOPHIA_ROOT), the
//! requested profiles under `profiles/OWNER/`, the reviewed dependency
//! manifests under `nim-deps/`, `inputs.env` (KEY=VALUE, absolute paths and
//! hex only, for a strict reader that never sources it) and
//! `physical-inputs.manifest` (crate::records encoding). `verify` requires
//! the manifest's expected sha256 and re-derives every identity.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::bemenu_artifact::{SignedTree, authorize, set_mode, signed_tree_under};
use crate::nim_deps::{FileEntry, Manifest, NOTE, inventory, writable_again};
use crate::package_desktop::{
    cargo, clean_checkout, git_text, private_dir, provisioned, resolve_lexically,
};
use crate::product_artifact::{Built, NimDeps, TOOLCHAIN_NOTE, build, nim_deps_option, product};
use crate::records::{Record, parse_text, relative_path, render};
use crate::{hex, pins, read, sha256};

const USAGE: &str = "usage: cargo xtask prepare-physical-inputs --sophia-root=/ABS --build-dir=/ABS \
                     --out=/ABS/NEW --sophia-features=native-session|atomic-scanout-live \
                     [--sophia-packages=sophia-cli[,sophia-wm-demo|sophia-conformance]] \
                     [--hagia=/ABS --hagia-commit=SHA --hagia-nim-deps=/ABS --hagia-nim-deps-sha256=SHA] \
                     [--narthex=/ABS --narthex-commit=SHA --narthex-nim-deps=/ABS --narthex-nim-deps-sha256=SHA] \
                     [--profile=OWNER:PATH ...] | verify --out=/ABS --manifest-sha256=SHA";
pub const MANIFEST: &str = "physical-inputs.manifest";
pub const ENV: &str = "inputs.env";
pub const SOPHIA_TREE: &str = "sophia-tree";
const SCHEMA: &str = "1";
const OPTIONS: [&str; 13] = [
    "sophia-root",
    "build-dir",
    "out",
    "sophia-features",
    "sophia-packages",
    "hagia",
    "hagia-commit",
    "hagia-nim-deps",
    "hagia-nim-deps-sha256",
    "narthex",
    "narthex-commit",
    "narthex-nim-deps",
    "narthex-nim-deps-sha256",
];
/// A product record's fields, in order.
const PRODUCT_KEYS: [&str; 9] = [
    "name",
    "commit",
    "tree",
    "signer",
    "nim_deps_sha256",
    "nim_config_sha256",
    "nim_config_read",
    "nim_stdlib_sha256",
    "nim_command",
];
pub const FEATURES: [&str; 2] = ["native-session", "atomic-scanout-live"];
pub const PACKAGES: [&str; 3] = [
    "sophia-cli",
    "sophia-cli,sophia-wm-demo",
    "sophia-cli,sophia-conformance",
];

pub fn run(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    run_with(repo, args, std::env::var("CARGO_HOME").ok().as_deref())
}

/// `run` with the caller's CARGO_HOME passed explicitly (tests).
pub fn run_with(
    repo: &Path,
    args: &[String],
    cargo_home: Option<&str>,
) -> Result<Vec<String>, String> {
    if args.first().map(String::as_str) == Some("verify") {
        let options =
            crate::product_artifact::options(&args[1..], &["out", "manifest-sha256"], USAGE)?;
        let get = |key: &str| {
            options
                .get(key)
                .copied()
                .ok_or_else(|| format!("verify needs --{key} (no default); {USAGE}"))
        };
        let sealed = verify(Path::new(get("out")?), get("manifest-sha256")?)?;
        return Ok(vec![format!(
            "physical_inputs status=verified manifest_sha256={} sophia={} dir={}",
            sealed.sha256,
            sealed.sophia_commit,
            sealed.out.display()
        )]);
    }
    let request = Request::parse(args)?;
    request.prepare(repo, cargo_home)
}

/// One named Nim product half.
struct Half<'a> {
    name: &'static str,
    repo: PathBuf,
    commit: &'a str,
    deps: NimDeps<'a>,
}

/// The validated command line.
pub struct Request<'a> {
    sophia_root: PathBuf,
    build_dir: PathBuf,
    out: PathBuf,
    features: &'a str,
    packages: Vec<&'static str>,
    halves: Vec<Half<'a>>,
    profiles: Vec<(String, String)>,
}

impl<'a> Request<'a> {
    pub fn parse(args: &'a [String]) -> Result<Self, String> {
        let mut profiles = Vec::new();
        let mut options = BTreeMap::new();
        for arg in args {
            match arg.strip_prefix("--profile=") {
                Some(value) => profiles.push(profile(value)?),
                None => {
                    for (key, value) in crate::product_artifact::options(
                        std::slice::from_ref(arg),
                        &OPTIONS,
                        USAGE,
                    )? {
                        if options.insert(key, value).is_some() {
                            return Err(format!("--{key} is repeated"));
                        }
                    }
                }
            }
        }
        if profiles
            .iter()
            .enumerate()
            .any(|(i, p)| profiles[..i].contains(p))
        {
            return Err("a --profile is repeated".into());
        }
        let get = |key: &str| {
            options
                .get(key)
                .copied()
                .ok_or_else(|| format!("--{key} is required (no default); {USAGE}"))
        };
        for key in [
            "sophia-root",
            "build-dir",
            "out",
            "hagia",
            "narthex",
            "hagia-nim-deps",
            "narthex-nim-deps",
        ] {
            if let Some(value) = options.get(key)
                && !value.starts_with('/')
            {
                return Err(format!("--{key} must be absolute: {value}"));
            }
        }
        let features = get("sophia-features")?;
        if !FEATURES.contains(&features) {
            return Err(format!(
                "--sophia-features must be one of {FEATURES:?}: {features:?}"
            ));
        }
        let packages = match options
            .get("sophia-packages")
            .copied()
            .unwrap_or("sophia-cli")
        {
            "sophia-cli" => vec!["sophia-cli"],
            "sophia-cli,sophia-wm-demo" => vec!["sophia-cli", "sophia-wm-demo"],
            "sophia-cli,sophia-conformance" => vec!["sophia-cli", "sophia-conformance"],
            other => {
                return Err(format!(
                    "--sophia-packages must be one of {PACKAGES:?}: {other:?}"
                ));
            }
        };
        let mut halves = Vec::new();
        for name in ["hagia", "narthex"] {
            let repo = options.get(name);
            let commit = options.get(format!("{name}-commit").as_str());
            let deps = nim_deps_option(&options, &format!("{name}-"))?;
            match (repo, commit, deps) {
                (None, None, None) => {}
                (Some(repo), Some(commit), Some(deps)) => {
                    if !hex(commit, 40) {
                        return Err(format!(
                            "--{name}-commit must be 40 lowercase hex: {commit:?}"
                        ));
                    }
                    halves.push(Half {
                        name,
                        repo: std::fs::canonicalize(repo)
                            .map_err(|e| format!("--{name} {repo}: {e}"))?,
                        commit,
                        deps,
                    });
                }
                _ => {
                    return Err(format!(
                        "--{name}, --{name}-commit, --{name}-nim-deps and --{name}-nim-deps-sha256 go together"
                    ));
                }
            }
        }
        for (owner, _) in &profiles {
            if !["sophia", "integration"].contains(&owner.as_str())
                && !halves.iter().any(|h| h.name == owner)
            {
                return Err(format!("--profile names {owner}, which is not staged"));
            }
        }
        let out = PathBuf::from(get("out")?);
        if !out.to_str().is_some_and(safe_value) {
            return Err(format!(
                "--out may hold only [A-Za-z0-9._/+-]: {}",
                out.display()
            ));
        }
        Ok(Self {
            sophia_root: std::fs::canonicalize(get("sophia-root")?)
                .map_err(|e| format!("--sophia-root: {e}"))?,
            build_dir: PathBuf::from(get("build-dir")?),
            out,
            features,
            packages,
            halves,
            profiles,
        })
    }

    fn prepare(&self, repo: &Path, cargo_home: Option<&str>) -> Result<Vec<String>, String> {
        let repo = std::fs::canonicalize(repo).map_err(|e| e.to_string())?;
        let mut sources = vec![&repo, &self.sophia_root];
        sources.extend(self.halves.iter().map(|h| &h.repo));
        for (key, path) in [("build-dir", &self.build_dir), ("out", &self.out)] {
            let resolved = resolve_lexically(path);
            for tree in &sources {
                if resolved.starts_with(tree) || tree.starts_with(&resolved) {
                    return Err(format!(
                        "--{key} must be outside every source tree: {}",
                        path.display()
                    ));
                }
            }
        }
        private_dir(&self.build_dir)?;
        if self.out.symlink_metadata().is_ok() {
            return Err(format!("--out already exists: {}", self.out.display()));
        }
        if !self.out.parent().is_some_and(Path::is_dir) {
            return Err(format!(
                "--out parent is not a directory: {}",
                self.out.display()
            ));
        }
        // The full provisioning marker before anything is staged or built.
        provisioned(&repo, cargo_home)?;
        clean_checkout(&repo, "Integration repository")?;
        let integration_commit = git_text(&repo, &["rev-parse", "--verify", "HEAD"])?
            .trim()
            .to_owned();
        let integration_signer = authorize(&repo, &integration_commit)?;
        clean_checkout(&self.sophia_root, "Sophia checkout")?;
        let head = git_text(&self.sophia_root, &["rev-parse", "--verify", "HEAD"])?;
        if head.trim() != pins::SOPHIA_REV {
            return Err(format!(
                "Sophia HEAD {} is not the pinned revision {}",
                head.trim(),
                pins::SOPHIA_REV
            ));
        }
        let sophia = signed_tree_under(
            &self.build_dir,
            &self.sophia_root,
            pins::SOPHIA_REV,
            "sophia",
        )?;
        let mut cargo_args = Vec::new();
        for package in &self.packages {
            cargo_args.extend(["-p", *package]);
        }
        let feature = format!("sophia-cli/{}", self.features);
        cargo_args.extend(["--features", feature.as_str()]);
        let target = self.build_dir.join("sophia-target");
        cargo(
            &sophia.tree_dir,
            &target,
            None,
            &cargo_args,
            &self.build_dir.join("sophia-build.log"),
            "build Sophia",
        )?;
        if crate::git_tree::inventory(&sophia.tree_dir)?.tree != sophia.tree {
            return Err("the Sophia build changed its staged tree".into());
        }
        let mut built = Vec::new();
        for half in &self.halves {
            built.push((
                half,
                build(
                    product(half.name)?,
                    &half.repo,
                    half.commit,
                    &self.build_dir,
                    Some(half.deps),
                )?,
            ));
        }
        // Profiles come from the staged trees only, after every proof.
        let integration = self
            .profiles
            .iter()
            .any(|(owner, _)| owner == "integration")
            .then(|| signed_tree_under(&self.build_dir, &repo, &integration_commit, "integration"))
            .transpose()?;
        let mut profiles = Vec::new();
        for (owner, path) in &self.profiles {
            let tree = if owner == "sophia" {
                &sophia
            } else if owner == "integration" {
                integration
                    .as_ref()
                    .expect("requested integration profile was staged")
            } else {
                &built
                    .iter()
                    .find(|(h, _)| h.name == owner.as_str())
                    .expect("validated owner")
                    .1
                    .tree
            };
            profiles.push((
                owner.clone(),
                path.clone(),
                read(&inside(&tree.tree_dir, path)?)?,
            ));
        }
        let binaries = self
            .packages
            .iter()
            .map(|p| {
                let name = match *p {
                    "sophia-cli" => "sophia",
                    "sophia-conformance" => "desktop_profile_probe",
                    _ => "sophia-wm-demo",
                };
                (name.to_owned(), target.join("release").join(name))
            })
            .chain(
                built
                    .iter()
                    .map(|(h, b)| (h.name.to_owned(), b.binary.clone())),
            )
            .collect::<Vec<_>>();
        let assembly = Assembly {
            out: &self.out,
            integration: (integration_commit.as_str(), integration_signer.as_str()),
            sophia: &sophia,
            features: self.features,
            packages: self.packages.join(","),
            built: &built.iter().map(|(h, b)| (h.name, b)).collect::<Vec<_>>(),
            binaries: &binaries,
            profiles: &profiles,
        };
        std::fs::create_dir(&self.out).map_err(|e| format!("{}: {e}", self.out.display()))?;
        match assembly.write() {
            Ok(digest) => Ok(vec![format!(
                "physical_inputs status=prepared manifest_sha256={digest} sophia={} integration={integration_commit} dir={}",
                pins::SOPHIA_REV,
                self.out.display()
            )]),
            Err(error) => {
                writable_again(&self.out);
                let _ = std::fs::remove_dir_all(&self.out);
                Err(error)
            }
        }
    }
}

/// Values written to inputs.env: absolute paths and hex, nothing a shell
/// or a strict reader could misread.
fn safe_value(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/+-".contains(&b))
}

/// `OWNER:PATH`, the owner a staged source, the path strictly inside it.
fn profile(value: &str) -> Result<(String, String), String> {
    let (owner, path) = value
        .split_once(':')
        .ok_or_else(|| format!("--profile must be OWNER:PATH: {value:?}"))?;
    if !["sophia", "hagia", "narthex", "integration"].contains(&owner) {
        return Err(format!(
            "--profile owner must be sophia, hagia, narthex or integration: {owner:?}"
        ));
    }
    relative_path(path)?;
    if !safe_value(path) {
        return Err(format!(
            "--profile path may hold only [A-Za-z0-9._/+-]: {path:?}"
        ));
    }
    Ok((owner.to_owned(), path.to_owned()))
}

/// A regular file strictly inside `root`: no component may be a link.
fn inside(root: &Path, relative: &str) -> Result<PathBuf, String> {
    relative_path(relative)?;
    let mut path = root.to_path_buf();
    let parts = relative.split('/').collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{relative}: {e}"))?;
        let last = index + 1 == parts.len();
        if meta.file_type().is_symlink() || (last && !meta.is_file()) || (!last && !meta.is_dir()) {
            return Err(format!(
                "{relative} is not a regular file inside its staged source"
            ));
        }
    }
    Ok(path)
}

struct Assembly<'a> {
    out: &'a Path,
    integration: (&'a str, &'a str),
    sophia: &'a SignedTree,
    features: &'a str,
    packages: String,
    built: &'a [(&'static str, &'a Built)],
    binaries: &'a [(String, PathBuf)],
    profiles: &'a [(String, String, Vec<u8>)],
}

impl Assembly<'_> {
    /// Fill the new output; returns the manifest's sha256.
    fn write(&self) -> Result<String, String> {
        let out = self.out;
        for dir in ["bin", "profiles", "nim-deps"] {
            std::fs::create_dir(out.join(dir)).map_err(|e| e.to_string())?;
        }
        for (name, binary) in self.binaries {
            let copy = out.join("bin").join(name);
            std::fs::copy(binary, &copy).map_err(|e| format!("copy {name}: {e}"))?;
            set_mode(&copy, 0o555)?;
        }
        copy_tree(&self.sophia.tree_dir, &out.join(SOPHIA_TREE))?;
        if crate::git_tree::inventory(&out.join(SOPHIA_TREE))?.tree != self.sophia.tree {
            return Err("the copied Sophia tree is not the pinned tree".into());
        }
        for (owner, path, bytes) in self.profiles {
            let dest = out.join("profiles").join(owner).join(path);
            std::fs::create_dir_all(dest.parent().ok_or("profile without a parent")?)
                .map_err(|e| e.to_string())?;
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
            set_mode(&dest, 0o444)?;
        }
        let mut header = vec![
            Record::of("physical-inputs").with("schema", SCHEMA),
            Record::of("integration")
                .with("commit", self.integration.0)
                .with("signer", self.integration.1),
            Record::of("sophia")
                .with("commit", pins::SOPHIA_REV)
                .with("tree", &self.sophia.tree)
                .with("signer", &self.sophia.signer)
                .with("features", self.features)
                .with("packages", &self.packages),
        ];
        for (name, built) in self.built {
            let nim = built
                .nim_deps
                .as_ref()
                .ok_or("a Nim half without reviewed dependencies")?;
            let file = out.join("nim-deps").join(format!("{name}.manifest"));
            std::fs::write(&file, &nim.reviewed.text).map_err(|e| e.to_string())?;
            set_mode(&file, 0o444)?;
            header.push(
                Record::of("product")
                    .with("name", name)
                    .with("commit", &nim.reviewed.manifest.source_commit)
                    .with("tree", &built.tree.tree)
                    .with("signer", &built.tree.signer)
                    .with("nim_deps_sha256", &nim.reviewed.sha256)
                    .with("nim_config_sha256", &nim.config_inventory_sha256)
                    .with("nim_config_read", nim.config_read())
                    .with("nim_stdlib_sha256", &nim.stdlib_inventory_sha256)
                    .with("nim_command", nim.command_line()),
            );
        }
        let identity = if self.built.is_empty() {
            "none"
        } else {
            TOOLCHAIN_NOTE
        };
        header.push(Record::of("build").with("toolchain_identity", identity));
        header.push(Record::of("note").with("text", NOTE));
        header.push(
            Record::of("sophia-tree")
                .with("path", SOPHIA_TREE)
                .with("git_tree", &self.sophia.tree),
        );
        let env = inputs_env(out, &header)?;
        std::fs::write(out.join(ENV), &env).map_err(|e| e.to_string())?;
        set_mode(&out.join(ENV), 0o444)?;
        let manifest = seal(out, header)?;
        std::fs::write(out.join(MANIFEST), &manifest).map_err(|e| e.to_string())?;
        set_mode(&out.join(MANIFEST), 0o444)?;
        read_only_tree(out)?;
        let digest = sha256(manifest.as_bytes());
        verify(out, &digest)?;
        Ok(digest)
    }
}

/// Copy a staged tree: regular files and directories only; read-only.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let dest = to.join(path.file_name().ok_or("entry without a name")?);
        if meta.is_dir() {
            copy_tree(&path, &dest)?;
        } else if meta.is_file() {
            std::fs::copy(&path, &dest).map_err(|e| format!("copy {}: {e}", path.display()))?;
            use std::os::unix::fs::PermissionsExt as _;
            set_mode(
                &dest,
                if meta.permissions().mode() & 0o100 != 0 {
                    0o555
                } else {
                    0o444
                },
            )?;
        } else {
            return Err(format!(
                "staged tree holds a non-regular entry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_only_tree(dir: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            read_only_tree(&path)?;
        }
    }
    set_mode(dir, 0o555)
}

fn field<'r>(records: &'r [Record], kind: &str, key: &str) -> Option<&'r str> {
    records
        .iter()
        .find(|r| r.kind == kind)
        .and_then(|r| r.fields.iter().find(|(k, _)| k == key))
        .map(|(_, v)| v.as_str())
}

/// inputs.env, derived only from the header records and the output path.
fn inputs_env(out: &Path, header: &[Record]) -> Result<String, String> {
    let dir = out
        .to_str()
        .filter(|d| safe_value(d))
        .ok_or("unsafe output path")?;
    let mut lines = vec![
        ("SOPHIA_PHYSICAL_INPUTS", dir.to_owned()),
        ("SOPHIA_ROOT", format!("{dir}/{SOPHIA_TREE}")),
        (
            "SOPHIA_COMMIT",
            field(header, "sophia", "commit")
                .ok_or("no sophia record")?
                .to_owned(),
        ),
        (
            "SOPHIA_INTEGRATION_COMMIT",
            field(header, "integration", "commit")
                .ok_or("no integration record")?
                .to_owned(),
        ),
        ("SOPHIA_BIN", format!("{dir}/bin/sophia")),
    ];
    if field(header, "sophia", "packages").is_some_and(|p| p.contains("sophia-wm-demo")) {
        lines.push(("SOPHIA_WM_DEMO_BIN", format!("{dir}/bin/sophia-wm-demo")));
    }
    if field(header, "sophia", "packages").is_some_and(|p| p.contains("sophia-conformance")) {
        lines.push((
            "SOPHIA_PROFILE_PROBE_BIN",
            format!("{dir}/bin/desktop_profile_probe"),
        ));
    }
    for record in header.iter().filter(|r| r.kind == "product") {
        let name = &record.fields[0].1;
        let commit = &record.fields[1].1;
        let (bin, key) = match name.as_str() {
            "hagia" => ("SOPHIA_HAGIA_BIN", "SOPHIA_HAGIA_COMMIT"),
            "narthex" => ("SOPHIA_NARTHEX_BIN", "SOPHIA_NARTHEX_COMMIT"),
            other => return Err(format!("unknown product {other}")),
        };
        lines.push((bin, format!("{dir}/bin/{name}")));
        lines.push((key, commit.clone()));
    }
    lines.push(("SOPHIA_PROFILE_DIR", format!("{dir}/profiles")));
    let mut text = String::new();
    for (key, value) in lines {
        if !safe_value(&value) {
            return Err(format!("{key} is not a safe value"));
        }
        text.push_str(&format!("{key}={value}\n"));
    }
    Ok(text)
}

/// The manifest text: the header, then one `file` record per regular file
/// outside `sophia-tree/` (which the header binds by its git tree).
pub fn seal(out: &Path, header: Vec<Record>) -> Result<String, String> {
    let mut records = header;
    for file in listed_files(out)? {
        records.push(
            Record::of("file")
                .with("path", &file.path)
                .with("mode", format!("{:04o}", file.mode))
                .with("size", file.size.to_string())
                .with("sha256", &file.sha256),
        );
    }
    render(&records)
}

fn listed_files(out: &Path) -> Result<Vec<FileEntry>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("non-UTF-8 entry")?
            .to_owned();
        if name == SOPHIA_TREE || name == MANIFEST {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            for mut file in inventory(&path, "")? {
                file.path = format!("{name}/{}", file.path);
                files.push(file);
            }
        } else if meta.is_file() {
            use std::os::unix::fs::PermissionsExt as _;
            files.push(FileEntry {
                package: String::new(),
                path: name,
                mode: meta.permissions().mode() & 0o7777,
                size: meta.len(),
                sha256: sha256(&read(&path)?),
            });
        } else {
            return Err(format!(
                "output holds a non-regular entry: {}",
                path.display()
            ));
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// A verified output.
#[derive(Debug)]
pub struct Sealed {
    pub out: PathBuf,
    pub sha256: String,
    pub sophia_commit: String,
    pub records: Vec<Record>,
}

/// Bind an output to the expected manifest sha256 (checked before parsing),
/// then re-derive: every file's record, no unlisted file, the Sophia tree's
/// git tree, and inputs.env exactly as the header and this path give it.
pub fn verify(out: &Path, expected: &str) -> Result<Sealed, String> {
    if !hex(expected, 64) {
        return Err(format!(
            "--manifest-sha256 must be 64 lowercase hex: {expected:?}"
        ));
    }
    if !out.is_absolute() || !std::fs::symlink_metadata(out).is_ok_and(|m| m.is_dir()) {
        return Err(format!(
            "physical inputs must be an absolute real directory: {}",
            out.display()
        ));
    }
    let manifest_path = out.join(MANIFEST);
    if !std::fs::symlink_metadata(&manifest_path).is_ok_and(|m| m.is_file()) {
        return Err(format!("{} has no regular {MANIFEST}", out.display()));
    }
    let bytes = read(&manifest_path)?;
    if sha256(&bytes) != expected {
        return Err(format!("{MANIFEST} is not the expected manifest"));
    }
    let text = String::from_utf8(bytes).map_err(|_| format!("{MANIFEST} is not UTF-8"))?;
    let records = parse_text(&text)?;
    let header_len = records
        .iter()
        .position(|r| r.kind == "file")
        .unwrap_or(records.len());
    let header = records[..header_len].to_vec();
    let kinds = header.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>();
    let products = kinds.iter().filter(|k| **k == "product").count();
    let mut expected_kinds = vec!["physical-inputs", "integration", "sophia"];
    expected_kinds.extend(std::iter::repeat_n("product", products));
    expected_kinds.extend(["build", "note", "sophia-tree"]);
    if kinds != expected_kinds {
        return Err(format!("{MANIFEST} header records are {kinds:?}"));
    }
    if header[0].expect(&["schema"])?[0] != SCHEMA {
        return Err(format!("{MANIFEST} has an unsupported schema"));
    }
    let integration = header[1].expect(&["commit", "signer"])?;
    let sophia = header[2].expect(&["commit", "tree", "signer", "features", "packages"])?;
    if !FEATURES.contains(&sophia[3]) || !PACKAGES.contains(&sophia[4]) {
        return Err(format!(
            "{MANIFEST} has unsupported Sophia features or packages"
        ));
    }
    for package in sophia[4].split(',') {
        use std::os::unix::fs::PermissionsExt as _;
        let binary = match package {
            "sophia-cli" => "sophia",
            "sophia-conformance" => "desktop_profile_probe",
            _ => "sophia-wm-demo",
        };
        let path = out.join("bin").join(binary);
        if !std::fs::symlink_metadata(&path)
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        {
            return Err(format!("missing regular executable {}", path.display()));
        }
    }
    if !hex(integration[0], 40) || sophia[0] != pins::SOPHIA_REV || !hex(sophia[1], 40) {
        return Err(format!("{MANIFEST} names a malformed or unpinned commit"));
    }
    let mut names = Vec::new();
    for record in &header[3..3 + products] {
        let v = record.expect(&PRODUCT_KEYS)?;
        if !["hagia", "narthex"].contains(&v[0])
            || names.contains(&v[0])
            || !hex(v[1], 40)
            || !hex(v[4], 64)
            || v[6].is_empty()
            || v[8].is_empty()
        {
            return Err(format!("{MANIFEST} has a malformed product record"));
        }
        let deps = read(&out.join("nim-deps").join(format!("{}.manifest", v[0])))?;
        if sha256(&deps) != v[4] {
            return Err(format!(
                "the {} dependency manifest is not the bound one",
                v[0]
            ));
        }
        // The staged installation the product was built from is the
        // reviewed one: configuration and stdlib identities agree.
        let deps = Manifest::parse(
            &String::from_utf8(deps).map_err(|_| "a dependency manifest is not UTF-8")?,
        )?;
        for (value, role) in [(v[5], "nim-config"), (v[7], "nim-lib")] {
            let reviewed = deps
                .toolchain
                .iter()
                .find(|r| r.kind == "tree" && r.fields[0].1 == role)
                .map(|r| r.fields[3].1.as_str());
            if reviewed != Some(value) || deps.product != v[0] || deps.source_commit != v[1] {
                return Err(format!(
                    "the {} build does not name its reviewed {role}",
                    v[0]
                ));
            }
        }
        names.push(v[0]);
    }
    let mut sorted = names.clone();
    sorted.sort_unstable();
    if sorted != names {
        return Err(format!("{MANIFEST} products are not sorted"));
    }
    header[3 + products].expect(&["toolchain_identity"])?;
    if header[4 + products].expect(&["text"])?[0] != NOTE {
        return Err(format!("{MANIFEST} note is not the toolchain statement"));
    }
    let tree = header[5 + products].expect(&["path", "git_tree"])?;
    if tree[0] != SOPHIA_TREE || tree[1] != sophia[1] {
        return Err(format!(
            "{MANIFEST} sophia-tree record is not the Sophia tree"
        ));
    }
    if crate::git_tree::inventory(&out.join(SOPHIA_TREE))?.tree != sophia[1] {
        return Err("sophia-tree is not the pinned Sophia tree".into());
    }
    // Every other file, re-derived; the whole text must be reproduced.
    if seal(out, header.clone())? != text {
        return Err(format!(
            "the files of {} are not the ones {MANIFEST} lists",
            out.display()
        ));
    }
    if read(&out.join(ENV))? != inputs_env(out, &header)?.into_bytes() {
        return Err(format!("{ENV} does not name this output"));
    }
    Ok(Sealed {
        out: out.to_path_buf(),
        sha256: expected.to_owned(),
        sophia_commit: sophia[0].to_owned(),
        records: header,
    })
}

/// The inputs.env keys a runner may read, and nothing else.
pub const ENV_KEYS: [&str; 12] = [
    "SOPHIA_PHYSICAL_INPUTS",
    "SOPHIA_ROOT",
    "SOPHIA_COMMIT",
    "SOPHIA_INTEGRATION_COMMIT",
    "SOPHIA_BIN",
    "SOPHIA_WM_DEMO_BIN",
    "SOPHIA_PROFILE_PROBE_BIN",
    "SOPHIA_HAGIA_BIN",
    "SOPHIA_HAGIA_COMMIT",
    "SOPHIA_NARTHEX_BIN",
    "SOPHIA_NARTHEX_COMMIT",
    "SOPHIA_PROFILE_DIR",
];

/// Exposed for tests: the manifest header records for a synthetic output.
pub fn header_for_tests(
    integration: &str,
    sophia_tree: &str,
    products: &[(&str, &str, &str, &str, &str)],
) -> Vec<Record> {
    let mut header = vec![
        Record::of("physical-inputs").with("schema", SCHEMA),
        Record::of("integration")
            .with("commit", integration)
            .with("signer", "ABCDEF"),
        Record::of("sophia")
            .with("commit", pins::SOPHIA_REV)
            .with("tree", sophia_tree)
            .with("signer", "ABCDEF")
            .with("features", "native-session")
            .with("packages", "sophia-cli"),
    ];
    for (name, commit, deps, config, stdlib) in products {
        header.push(
            Record::of("product")
                .with("name", name)
                .with("commit", commit)
                .with("tree", "1".repeat(40))
                .with("signer", "ABCDEF")
                .with("nim_deps_sha256", deps)
                .with("nim_config_sha256", config)
                .with("nim_config_read", "config/nim.cfg:x")
                .with("nim_stdlib_sha256", stdlib)
                .with("nim_command", "/b/nim/bin/nim c"),
        );
    }
    let identity = if products.is_empty() {
        "none"
    } else {
        TOOLCHAIN_NOTE
    };
    header.push(Record::of("build").with("toolchain_identity", identity));
    header.push(Record::of("note").with("text", NOTE));
    header.push(
        Record::of("sophia-tree")
            .with("path", SOPHIA_TREE)
            .with("git_tree", sophia_tree),
    );
    header
}

/// Exposed for tests: write inputs.env for a synthetic output.
pub fn write_env_for_tests(out: &Path, header: &[Record]) -> Result<(), String> {
    std::fs::write(out.join(ENV), inputs_env(out, header)?).map_err(|e| e.to_string())
}

/// Exposed for tests: `OWNER:PATH` validation and the in-tree lookup.
pub fn profile_for_tests(value: &str, root: &Path) -> Result<PathBuf, String> {
    let (_, path) = profile(value)?;
    inside(root, &path)
}
