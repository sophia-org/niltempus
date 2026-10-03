//! Bound Nim dependency closures for the Hagia, Narthex and kleis builds.
//!
//! A product's Nim dependencies are named by a REVIEWED dependency manifest
//! whose sha256 the operator or root supplies independently. The manifest
//! records the full transitive closure (every package's provenance and
//! complete file inventory) and the host toolchain identity. Nothing here
//! picks a version: `xtask nim-deps draft` resolves only explicit `--pin`
//! choices, reading each `.nimble` file as DATA (nimble is never executed),
//! and writes a `status=draft` file for root's review. A draft authorizes
//! nothing; the builder refuses it.
//!
//! Host-toolchain identity (nim, its standard library and installation
//! config, gcc, cc1, as, ld, bwrap and the owning host packages) is RECORDED
//! and re-checked before and after each build. It is not a fully
//! reproducible closure: the host's toolchain is identified, not rebuilt.
//!
//! Manifest records (crate::records encoding, this order, each list sorted):
//!
//! ```text
//! nim-deps schema=1 status=draft|reviewed
//! for product=hagia source_commit=<40> source_tree=<40> store=/ABS
//! tool role=as|bwrap|cc1|gcc|ld|nim path=/ABS resolved=/ABS version="..." sha256=<64>
//! tree role=nim-config|nim-lib path=/ABS files=<n> inventory_sha256=<64>
//! hostpkg path=/ABS pkgver=<name-version>
//! note text="..."
//! package name= version= dir= checksum=<40> origin_url= origin_vcs_revision= requires="..." files=<n> inventory_sha256=<64>
//! file package= path= mode=<0NNN> size=<n> sha256=<64>
//! ```
//!
//! A package's `inventory_sha256` is the sha256 of its own encoded `file`
//! lines (each with its newline); a `tree` record's is the same digest over
//! lines naming the tree's role as the package.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::bemenu_artifact::{absolute, bounded, set_mode, text};
use crate::records::{Record, parse_text, relative_path, render};
use crate::{hex, read, sha256};

pub const SCHEMA: &str = "1";
pub const NOTE: &str =
    "host toolchain identity is recorded and re-checked; it is not a fully reproducible closure";
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const USAGE: &str = "usage: cargo xtask nim-deps draft --store=/ABS --source=/ABS/REPO \
                     --commit=<40 hex> --product=hagia|narthex|kleis --nim=/ABS --nim-lib=/ABS \
                     --gcc=/ABS --bwrap=/ABS --build-dir=/ABS --pin=NAME=VERSION ... --out=/ABS/NEW-FILE";

// ---------------------------------------------------------------- versions

fn version(value: &str) -> Option<Vec<u64>> {
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect()
}

fn compare(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let length = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..length)
        .map(|i| at(a, i).cmp(&at(b, i)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

// ------------------------------------------------------------ requirements

/// One `requires` entry: a package name and its version constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub name: String,
    pub constraints: Vec<(String, Vec<u64>)>,
    pub raw: String,
}

impl Requirement {
    pub fn allows(&self, candidate: &[u64]) -> bool {
        self.constraints.iter().all(|(op, bound)| {
            let order = compare(candidate, bound);
            match op.as_str() {
                ">=" => order.is_ge(),
                ">" => order.is_gt(),
                "<=" => order.is_le(),
                "<" => order.is_lt(),
                "==" => order.is_eq(),
                _ => false,
            }
        })
    }
}

fn unsupported(expression: &str, why: &str) -> String {
    format!("unsupported requires expression `{expression}`: {why}")
}

/// One requirement string: `name`, or `name OP version` joined by `&`, with
/// OP one of >= > <= < ==. Anything else (`^=`, `~=`, `#head`, URLs, `@`)
/// fails, naming the expression.
pub fn parse_requirement(raw: &str) -> Result<Requirement, String> {
    let spec = raw.trim();
    let name_end = spec
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .unwrap_or(spec.len());
    let name = &spec[..name_end];
    if name.is_empty() || name.starts_with(['.', '-']) {
        return Err(unsupported(raw, "no plain package name"));
    }
    let rest = spec[name_end..].trim();
    let mut constraints = Vec::new();
    if !rest.is_empty() {
        for part in rest.split('&') {
            let part = part.trim();
            let op = [">=", "<=", "==", ">", "<"]
                .into_iter()
                .find(|op| part.starts_with(op))
                .ok_or_else(|| {
                    unsupported(raw, "only >=, >, <=, < and == constraints are supported")
                })?;
            let bound = version(part[op.len()..].trim())
                .ok_or_else(|| unsupported(raw, "a constraint needs a dotted numeric version"))?;
            constraints.push((op.to_owned(), bound));
        }
    }
    Ok(Requirement {
        name: name.to_owned(),
        constraints,
        raw: spec.to_owned(),
    })
}

/// Strip a trailing `#` comment outside string literals.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            '\\' if quoted && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !escaped => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
        escaped = false;
    }
    line
}

fn is_requires_statement(trimmed: &str) -> bool {
    trimmed
        .strip_prefix("requires")
        .is_some_and(|rest| rest.starts_with([' ', '(', '"', '\t']))
}

/// The top-level `requires` statements of a `.nimble` file, read as data.
/// A `requires` inside any block (when/if/feature/task), a parenthesized or
/// non-literal form, or an `include` fails, naming the expression.
pub fn nimble_requires(text: &str) -> Result<Vec<Requirement>, String> {
    let lines = text.lines().map(strip_comment).collect::<Vec<_>>();
    let mut requirements = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        index += 1;
        if trimmed.starts_with("include ") {
            return Err(unsupported(
                trimmed,
                "a .nimble include cannot be read as data",
            ));
        }
        if !is_requires_statement(trimmed) {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(unsupported(
                trimmed,
                "requires inside a block is conditional",
            ));
        }
        let mut statement = trimmed["requires".len()..].trim().to_owned();
        while statement.ends_with(',') {
            let next = lines
                .get(index)
                .ok_or_else(|| unsupported(&statement, "continues past the end of the file"))?;
            index += 1;
            statement.push(' ');
            statement.push_str(next.trim());
        }
        for literal in string_list(&statement)? {
            requirements.push(parse_requirement(&literal)?);
        }
    }
    Ok(requirements)
}

/// `"a", "b >= 1"`: comma-separated plain string literals only.
fn string_list(statement: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = statement.trim();
    loop {
        let body = rest
            .strip_prefix('"')
            .filter(|b| !b.starts_with("\"\""))
            .ok_or_else(|| unsupported(statement, "only plain string literals are supported"))?;
        let end = body
            .find('"')
            .ok_or_else(|| unsupported(statement, "unterminated string"))?;
        let literal = &body[..end];
        if literal.contains('\\') {
            return Err(unsupported(statement, "escapes are not supported"));
        }
        out.push(literal.to_owned());
        rest = body[end + 1..].trim();
        if rest.is_empty() {
            return Ok(out);
        }
        rest = rest
            .strip_prefix(',')
            .ok_or_else(|| unsupported(statement, "literals must be comma-separated"))?
            .trim();
    }
}

// --------------------------------------------------------------- inventory

/// One file of a package or tree inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub package: String,
    pub path: String,
    pub mode: u32,
    pub size: u64,
    pub sha256: String,
}

impl FileEntry {
    fn record(&self) -> Record {
        Record::of("file")
            .with("package", &self.package)
            .with("path", &self.path)
            .with("mode", format!("{:04o}", self.mode))
            .with("size", self.size.to_string())
            .with("sha256", &self.sha256)
    }
}

/// Every regular file below `root`, sorted by path. Symlinks and special
/// files are refused; the root itself must be a real directory.
pub fn inventory(root: &Path, package: &str) -> Result<Vec<FileEntry>, String> {
    let meta = std::fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} is not a real directory", root.display()));
    }
    let mut files = Vec::new();
    walk(root, root, package, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn walk(root: &Path, dir: &Path, package: &str, files: &mut Vec<FileEntry>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let meta =
            std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            walk(root, &path, package, files)?;
        } else if meta.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path under {}", root.display()))?
                .to_owned();
            relative_path(&relative)?;
            files.push(FileEntry {
                package: package.to_owned(),
                path: relative,
                mode: meta.permissions().mode() & 0o7777,
                size: meta.len(),
                sha256: sha256(&read(&path)?),
            });
        } else {
            return Err(format!(
                "only regular files and directories are allowed: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

/// The digest of a package's (or tree's) encoded `file` lines.
pub fn inventory_sha256(files: &[FileEntry]) -> Result<String, String> {
    Ok(sha256(
        render(&files.iter().map(FileEntry::record).collect::<Vec<_>>())?.as_bytes(),
    ))
}

// --------------------------------------------------------------- toolchain

/// The recorded host toolchain: `tool`, `tree` and `hostpkg` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    pub records: Vec<Record>,
    pub nim_version: Vec<u64>,
}

impl Toolchain {
    /// Refuse changed executables before the slower inventory and package
    /// probes. This is an early refusal only; full identity checks still run.
    pub fn check_executables(&self) -> Result<(), String> {
        for record in self.records.iter().filter(|r| r.kind == "tool") {
            let fields = record.expect(&["role", "path", "resolved", "version", "sha256"])?;
            let (role, path, expected_path, expected_hash) =
                (fields[0], fields[1], fields[2], fields[4]);
            let resolved = regular(Path::new(path), role)?;
            let actual_hash = sha256(&read(&resolved)?);
            if resolved != Path::new(expected_path) || actual_hash != expected_hash {
                return Err(format!(
                    "reviewed toolchain changed: {role} at {path}; expected resolved={expected_path} sha256={expected_hash}; actual resolved={} sha256={actual_hash}; review a new dependency manifest and update its configured digest",
                    resolved.display()
                ));
            }
        }
        Ok(())
    }

    pub fn tool(&self, role: &str) -> Result<PathBuf, String> {
        self.records
            .iter()
            .find(|r| r.kind == "tool" && r.fields.first().is_some_and(|(_, v)| v == role))
            .and_then(|r| r.fields.get(2))
            .map(|(_, resolved)| PathBuf::from(resolved))
            .ok_or_else(|| format!("the toolchain has no {role}"))
    }

    pub fn tree(&self, role: &str) -> Result<PathBuf, String> {
        self.records
            .iter()
            .find(|r| r.kind == "tree" && r.fields.first().is_some_and(|(_, v)| v == role))
            .and_then(|r| r.fields.get(1))
            .map(|(_, path)| PathBuf::from(path))
            .ok_or_else(|| format!("the toolchain has no {role}"))
    }
}

fn first_line(bytes: Vec<u8>) -> Result<String, String> {
    Ok(text(bytes)?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned())
}

fn probe_command(program: &Path, path_dir: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", path_dir)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null());
    command
}

fn regular(path: &Path, what: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{what} must be absolute: {}", path.display()));
    }
    let resolved =
        std::fs::canonicalize(path).map_err(|e| format!("{what} {}: {e}", path.display()))?;
    if !std::fs::metadata(&resolved).is_ok_and(|m| m.is_file()) {
        return Err(format!("{what} is not a regular file: {}", path.display()));
    }
    Ok(resolved)
}

fn owner(path: &Path) -> Result<String, String> {
    let output = first_line(bounded(
        Command::new("xbps-query")
            .arg("-o")
            .arg(path)
            .env_clear()
            .env("LC_ALL", "C"),
        PROBE_TIMEOUT,
        "xbps-query -o",
    )?)?;
    let (pkgver, rest) = output
        .split_once(": ")
        .ok_or_else(|| format!("no host package owns {}", path.display()))?;
    if !rest.starts_with(&*path.to_string_lossy()) || pkgver.contains(' ') || pkgver.is_empty() {
        return Err(format!(
            "unexpected host package answer for {}: {output:?}",
            path.display()
        ));
    }
    Ok(pkgver.to_owned())
}

/// Identify the host toolchain from explicit paths. `as` and `ld` are what
/// gcc resolves (`-print-prog-name`), searched only in gcc's own directory,
/// which is also the build's only PATH entry.
pub fn probe_toolchain(
    nim: &Path,
    lib: &Path,
    gcc: &Path,
    bwrap: &Path,
) -> Result<Toolchain, String> {
    let nim_resolved = regular(nim, "--nim")?;
    let gcc_resolved = regular(gcc, "--gcc")?;
    let bwrap_resolved = regular(bwrap, "--bwrap")?;
    let gcc_dir = gcc_resolved
        .parent()
        .ok_or("gcc has no directory")?
        .to_path_buf();
    let prog = |name: &str| -> Result<(PathBuf, PathBuf), String> {
        let answer = first_line(bounded(
            probe_command(&gcc_resolved, &gcc_dir).arg(format!("-print-prog-name={name}")),
            PROBE_TIMEOUT,
            "gcc -print-prog-name",
        )?)?;
        let named = if answer.starts_with('/') {
            PathBuf::from(&answer)
        } else if !answer.is_empty() && !answer.contains('/') {
            gcc_dir.join(&answer)
        } else {
            return Err(format!("gcc names {name} ambiguously: {answer:?}"));
        };
        Ok((named.clone(), regular(&named, name)?))
    };
    let version_of = |program: &Path, args: &[&str]| -> Result<String, String> {
        first_line(bounded(
            probe_command(program, &gcc_dir).args(args),
            PROBE_TIMEOUT,
            "toolchain --version",
        )?)
    };
    let nim_line = version_of(&nim_resolved, &["--version"])?;
    let nim_version = nim_line
        .strip_prefix("Nim Compiler Version ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(version)
        .ok_or_else(|| format!("unrecognized nim version line {nim_line:?}"))?;
    let mut tools = vec![("nim", nim.to_path_buf(), nim_resolved.clone(), nim_line)];
    tools.push((
        "gcc",
        gcc.to_path_buf(),
        gcc_resolved.clone(),
        version_of(&gcc_resolved, &["--version"])?,
    ));
    tools.push((
        "bwrap",
        bwrap.to_path_buf(),
        bwrap_resolved.clone(),
        version_of(&bwrap_resolved, &["--version"])?,
    ));
    // cc1 reports its version only on stderr; gcc names the version and
    // target it drives, and the sha256 binds the binary itself.
    let cc1_line = format!(
        "gcc -dumpfullversion {} -dumpmachine {}",
        version_of(&gcc_resolved, &["-dumpfullversion"])?,
        version_of(&gcc_resolved, &["-dumpmachine"])?
    );
    let (named, resolved) = prog("cc1")?;
    tools.push(("cc1", named, resolved, cc1_line));
    for name in ["as", "ld"] {
        let (named, resolved) = prog(name)?;
        let line = version_of(&resolved, &["--version"])?;
        tools.push((name, named, resolved, line));
    }
    tools.sort_by(|a, b| a.0.cmp(b.0));
    let mut records = Vec::new();
    let mut owned = BTreeSet::new();
    for (role, path, resolved, line) in &tools {
        records.push(
            Record::of("tool")
                .with("role", role)
                .with("path", path.to_string_lossy())
                .with("resolved", resolved.to_string_lossy())
                .with("version", line)
                .with("sha256", sha256(&read(resolved)?)),
        );
        if *role != "bwrap" {
            owned.insert(resolved.to_string_lossy().into_owned());
        }
    }
    // The installation config lives beside the compiler's bin directory.
    let config = nim_resolved
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("config"))
        .ok_or("nim has no installation prefix")?;
    let lib =
        std::fs::canonicalize(lib).map_err(|e| format!("--nim-lib {}: {e}", lib.display()))?;
    for (role, dir) in [("nim-config", &config), ("nim-lib", &lib)] {
        let files = inventory(dir, role)?;
        records.push(
            Record::of("tree")
                .with("role", role)
                .with("path", dir.to_string_lossy())
                .with("files", files.len().to_string())
                .with("inventory_sha256", &inventory_sha256(&files)?),
        );
    }
    for name in ["libc.so.6", "libc.so"] {
        let answer = first_line(bounded(
            probe_command(&gcc_resolved, &gcc_dir).arg(format!("-print-file-name={name}")),
            PROBE_TIMEOUT,
            "gcc -print-file-name",
        )?)?;
        owned.insert(
            regular(Path::new(&answer), name)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    owned.insert(
        regular(&lib.join("system.nim"), "nim stdlib")?
            .to_string_lossy()
            .into_owned(),
    );
    for path in owned {
        records.push(
            Record::of("hostpkg")
                .with("path", &path)
                .with("pkgver", &owner(Path::new(&path))?),
        );
    }
    Ok(Toolchain {
        records,
        nim_version,
    })
}

// ---------------------------------------------------------------- manifest

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub dir: String,
    pub checksum: String,
    pub origin_url: String,
    pub origin_vcs_revision: String,
    pub requires: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub status: String,
    pub product: String,
    pub source_commit: String,
    pub source_tree: String,
    pub store: PathBuf,
    pub toolchain: Vec<Record>,
    pub packages: Vec<Package>,
}

const PACKAGE_KEYS: [&str; 9] = [
    "name",
    "version",
    "dir",
    "checksum",
    "origin_url",
    "origin_vcs_revision",
    "requires",
    "files",
    "inventory_sha256",
];

impl Manifest {
    pub fn render(&self) -> Result<String, String> {
        let mut records = vec![
            Record::of("nim-deps")
                .with("schema", SCHEMA)
                .with("status", &self.status),
            Record::of("for")
                .with("product", &self.product)
                .with("source_commit", &self.source_commit)
                .with("source_tree", &self.source_tree)
                .with("store", self.store.to_string_lossy()),
        ];
        records.extend(self.toolchain.iter().cloned());
        records.push(Record::of("note").with("text", NOTE));
        for p in &self.packages {
            records.push(
                Record::of("package")
                    .with("name", &p.name)
                    .with("version", &p.version)
                    .with("dir", &p.dir)
                    .with("checksum", &p.checksum)
                    .with("origin_url", &p.origin_url)
                    .with("origin_vcs_revision", &p.origin_vcs_revision)
                    .with("requires", &p.requires)
                    .with("files", p.files.len().to_string())
                    .with("inventory_sha256", &inventory_sha256(&p.files)?),
            );
        }
        for p in &self.packages {
            records.extend(p.files.iter().map(FileEntry::record));
        }
        render(&records)
    }

    /// Parse and validate a manifest. It must be canonical: re-rendering the
    /// parsed content reproduces the text exactly (order, sorting, counts
    /// and digests included).
    pub fn parse(text: &str) -> Result<Self, String> {
        let records = parse_text(text)?;
        let mut at = records.iter().peekable();
        let header = at.next().ok_or("empty dependency manifest")?;
        if header.kind != "nim-deps" {
            return Err("dependency manifest must start with a nim-deps record".into());
        }
        let [schema, status] = header.expect(&["schema", "status"])?[..] else {
            unreachable!()
        };
        if schema != SCHEMA {
            return Err(format!("unsupported dependency manifest schema {schema:?}"));
        }
        if status != "draft" && status != "reviewed" {
            return Err(format!(
                "dependency manifest status must be draft or reviewed: {status:?}"
            ));
        }
        let target = at
            .next()
            .filter(|r| r.kind == "for")
            .ok_or("missing for record")?;
        let [product, source_commit, source_tree, store] =
            target.expect(&["product", "source_commit", "source_tree", "store"])?[..]
        else {
            unreachable!()
        };
        if !hex(source_commit, 40) || !hex(source_tree, 40) {
            return Err("for record needs 40-hex source_commit and source_tree".into());
        }
        if !store.starts_with('/') {
            return Err("for record store must be absolute".into());
        }
        let mut toolchain = Vec::new();
        while let Some(record) =
            at.next_if(|r| ["tool", "tree", "hostpkg"].contains(&r.kind.as_str()))
        {
            let keys: &[&str] = match record.kind.as_str() {
                "tool" => &["role", "path", "resolved", "version", "sha256"],
                "tree" => &["role", "path", "files", "inventory_sha256"],
                _ => &["path", "pkgver"],
            };
            record.expect(keys)?;
            toolchain.push(record.clone());
        }
        let note = at
            .next()
            .filter(|r| r.kind == "note")
            .ok_or("missing note record")?;
        if note.expect(&["text"])?[0] != NOTE {
            return Err("the note record must state that host-toolchain identity is not a reproducible closure".into());
        }
        let mut packages: Vec<Package> = Vec::new();
        let mut declared = Vec::new();
        while let Some(record) = at.next_if(|r| r.kind == "package") {
            let v = record.expect(&PACKAGE_KEYS)?;
            if packages.iter().any(|p| p.name == v[0]) {
                return Err(format!("package {} is listed twice", v[0]));
            }
            relative_path(v[2])?;
            if v[2].contains('/') || !hex(v[3], 40) || version(v[1]).is_none() {
                return Err(format!("malformed package record for {}", v[0]));
            }
            declared.push((v[7].to_owned(), v[8].to_owned()));
            packages.push(Package {
                name: v[0].to_owned(),
                version: v[1].to_owned(),
                dir: v[2].to_owned(),
                checksum: v[3].to_owned(),
                origin_url: v[4].to_owned(),
                origin_vcs_revision: v[5].to_owned(),
                requires: v[6].to_owned(),
                files: Vec::new(),
            });
        }
        let mut seen = BTreeSet::new();
        for record in at {
            if record.kind != "file" {
                return Err(format!("unexpected {} record", record.kind));
            }
            let v = record.expect(&["package", "path", "mode", "size", "sha256"])?;
            relative_path(v[1])?;
            if !seen.insert((v[0].to_owned(), v[1].to_owned())) {
                return Err(format!("file {}:{} is listed twice", v[0], v[1]));
            }
            let mode = u32::from_str_radix(v[2], 8)
                .ok()
                .filter(|m| v[2].len() == 4 && *m <= 0o7777)
                .ok_or_else(|| format!("malformed mode {:?}", v[2]))?;
            let size = v[3]
                .parse()
                .map_err(|_| format!("malformed size {:?}", v[3]))?;
            if !hex(v[4], 64) {
                return Err(format!("malformed sha256 for {}:{}", v[0], v[1]));
            }
            packages
                .iter_mut()
                .find(|p| p.name == v[0])
                .ok_or_else(|| format!("file names an unlisted package {}", v[0]))?
                .files
                .push(FileEntry {
                    package: v[0].to_owned(),
                    path: v[1].to_owned(),
                    mode,
                    size,
                    sha256: v[4].to_owned(),
                });
        }
        for (package, (count, digest)) in packages.iter().zip(&declared) {
            if package.files.len().to_string() != *count
                || inventory_sha256(&package.files)? != *digest
            {
                return Err(format!(
                    "package {} inventory does not match its files",
                    package.name
                ));
            }
        }
        // Every list strictly sorted: toolchain records by kind then role or
        // path, packages by name, each package's files by path.
        let order = |r: &Record| (r.kind.clone(), r.fields[0].1.clone());
        let sorted = |keys: Vec<(String, String)>| keys.windows(2).all(|w| w[0] < w[1]);
        let kind_rank = |r: &Record| {
            ["tool", "tree", "hostpkg"]
                .iter()
                .position(|k| *k == r.kind)
        };
        if !toolchain
            .windows(2)
            .all(|w| (kind_rank(&w[0]), order(&w[0]).1) < (kind_rank(&w[1]), order(&w[1]).1))
            || !sorted(
                packages
                    .iter()
                    .map(|p| (p.name.clone(), String::new()))
                    .collect(),
            )
            || !packages.iter().all(|p| {
                sorted(
                    p.files
                        .iter()
                        .map(|f| (f.path.clone(), String::new()))
                        .collect(),
                )
            })
        {
            return Err("dependency manifest lists are not strictly sorted".into());
        }
        let manifest = Self {
            status: status.to_owned(),
            product: product.to_owned(),
            source_commit: source_commit.to_owned(),
            source_tree: source_tree.to_owned(),
            store: PathBuf::from(store),
            toolchain,
            packages,
        };
        // Canonical order (sorted lists, fixed record order) is enforced by
        // requiring the exact re-rendering.
        if manifest.render()? != text {
            return Err("dependency manifest is not in canonical order".into());
        }
        Ok(manifest)
    }

    pub fn toolchain_view(&self) -> Result<Toolchain, String> {
        let nim_line = self
            .toolchain
            .iter()
            .find(|r| r.kind == "tool" && r.fields[0].1 == "nim")
            .map(|r| r.fields[3].1.clone())
            .ok_or("the manifest records no nim")?;
        let nim_version = nim_line
            .strip_prefix("Nim Compiler Version ")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(version)
            .ok_or("the manifest's nim version is unrecognized")?;
        Ok(Toolchain {
            records: self.toolchain.clone(),
            nim_version,
        })
    }
}

// ------------------------------------------------------------------ resolve

/// Resolve `root` requirements against explicit pins in `store`. Every
/// non-nim requirement, transitively, must be pinned; exactly one store
/// directory must match each pin; every pin must satisfy every constraint
/// on its package and be used. Nothing is ever chosen implicitly.
pub fn resolve(
    store: &Path,
    root: &[Requirement],
    pins: &BTreeMap<String, String>,
    nim_version: &[u64],
) -> Result<Vec<Package>, String> {
    let entries = store_entries(store)?;
    let mut queue = root
        .iter()
        .map(|r| (r.clone(), "the product".to_owned()))
        .collect::<VecDeque<_>>();
    let mut chosen: BTreeMap<String, Package> = BTreeMap::new();
    while let Some((requirement, from)) = queue.pop_front() {
        if requirement.name == "nim" {
            if !requirement.allows(nim_version) {
                return Err(format!(
                    "{from} requires {:?}; the recorded nim is {}",
                    requirement.raw,
                    nim_version
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(".")
                ));
            }
            continue;
        }
        let candidates = entries
            .iter()
            .filter(|e| e.starts_with(&format!("{}-", requirement.name)))
            .cloned()
            .collect::<Vec<_>>();
        let pinned = pins.get(&requirement.name).ok_or_else(|| {
            format!(
                "{from} requires {:?}, which is not pinned (--pin={}=VERSION); store candidates: {candidates:?}",
                requirement.raw, requirement.name
            )
        })?;
        let pinned_version = version(pinned)
            .ok_or_else(|| format!("malformed pin {}={pinned}", requirement.name))?;
        if !requirement.allows(&pinned_version) {
            return Err(format!(
                "{from} requires {:?}; the pin {}={pinned} does not satisfy it",
                requirement.raw, requirement.name
            ));
        }
        if chosen.contains_key(&requirement.name) {
            continue;
        }
        let matching = candidates
            .iter()
            .filter(|e| {
                e.strip_prefix(&format!("{}-{pinned}-", requirement.name))
                    .is_some_and(|checksum| hex(checksum, 40))
            })
            .collect::<Vec<_>>();
        let [dir] = matching[..] else {
            return Err(format!(
                "pin {}={pinned} matches {} store directories (exactly one required): {matching:?}",
                requirement.name,
                matching.len()
            ));
        };
        let package = read_package(store, dir, &requirement.name, pinned)?;
        for dependency in nimble_requires(&package.requires_text)? {
            queue.push_back((dependency, format!("{}@{pinned}", requirement.name)));
        }
        chosen.insert(requirement.name.clone(), package.package);
    }
    let unused = pins
        .keys()
        .filter(|name| !chosen.contains_key(*name))
        .collect::<Vec<_>>();
    if !unused.is_empty() {
        return Err(format!("pins not in the closure: {unused:?}"));
    }
    Ok(chosen.into_values().collect())
}

fn store_entries(store: &Path) -> Result<Vec<String>, String> {
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(store).map_err(|e| format!("--store {}: {e}", store.display()))?
    {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        entries.push(name.into_string().map_err(|_| "non-UTF-8 store entry")?);
    }
    entries.sort();
    Ok(entries)
}

struct ReadPackage {
    package: Package,
    requires_text: String,
}

fn read_package(store: &Path, dir: &str, name: &str, pinned: &str) -> Result<ReadPackage, String> {
    let root = store.join(dir);
    let files = inventory(&root, name)?;
    let nimble_name = format!("{name}.nimble");
    let nimble = files
        .iter()
        .find(|f| f.path == nimble_name)
        .ok_or_else(|| format!("{dir} has no {nimble_name}"))?;
    let requires_text = String::from_utf8(read(&root.join(&nimble.path))?)
        .map_err(|_| format!("{dir}/{nimble_name} is not UTF-8"))?;
    let requires = nimble_requires(&requires_text)?
        .into_iter()
        .map(|r| r.raw)
        .collect::<Vec<_>>()
        .join(", ");
    let meta: serde_json::Value = serde_json::from_slice(&read(&root.join("nimblemeta.json"))?)
        .map_err(|e| format!("{dir}/nimblemeta.json: {e}"))?;
    if meta["version"] != 1 {
        return Err(format!("{dir}/nimblemeta.json has an unsupported version"));
    }
    let data = &meta["metaData"];
    let text_field = |key: &str| {
        data[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{dir}/nimblemeta.json has no {key}"))
    };
    let special = data["specialVersions"]
        .as_array()
        .ok_or_else(|| format!("{dir}/nimblemeta.json has no specialVersions"))?;
    if !special.iter().any(|v| v.as_str() == Some(pinned)) {
        return Err(format!(
            "{dir}/nimblemeta.json does not name version {pinned}"
        ));
    }
    // nimble's directory checksum covers the downloaded repository, which
    // the install does not keep; it is recorded as provenance. The installed
    // file set must be exactly the one nimble recorded.
    let listed = data["files"]
        .as_array()
        .ok_or_else(|| format!("{dir}/nimblemeta.json has no files"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| s.strip_prefix('/'))
                .map(str::to_owned)
                .ok_or_else(|| format!("{dir}/nimblemeta.json has a malformed file entry"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let present = files
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| p != "nimblemeta.json")
        .collect::<BTreeSet<_>>();
    if listed != present {
        return Err(format!(
            "{dir} does not hold exactly nimblemeta.json's files (extra {:?}, missing {:?})",
            present.difference(&listed).collect::<Vec<_>>(),
            listed.difference(&present).collect::<Vec<_>>()
        ));
    }
    Ok(ReadPackage {
        package: Package {
            name: name.to_owned(),
            version: pinned.to_owned(),
            dir: dir.to_owned(),
            checksum: dir.rsplit('-').next().unwrap_or_default().to_owned(),
            origin_url: text_field("url")?,
            origin_vcs_revision: text_field("vcsRevision")?,
            requires,
            files,
        },
        requires_text,
    })
}

// -------------------------------------------------------------------- draft

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    match args.split_first() {
        Some((command, rest)) if command == "draft" => draft(rest),
        _ => Err(USAGE.into()),
    }
}

fn draft(args: &[String]) -> Result<Vec<String>, String> {
    let mut options = BTreeMap::new();
    let mut pins = BTreeMap::new();
    for arg in args {
        let (key, value) = arg
            .strip_prefix("--")
            .and_then(|a| a.split_once('='))
            .ok_or_else(|| format!("unexpected argument {arg:?}; {USAGE}"))?;
        if key == "pin" {
            let (name, pinned) = value
                .split_once('=')
                .filter(|(n, v)| !n.is_empty() && version(v).is_some())
                .ok_or_else(|| format!("--pin must be NAME=VERSION: {value:?}"))?;
            if pins.insert(name.to_owned(), pinned.to_owned()).is_some() {
                return Err(format!("--pin {name} is repeated"));
            }
            continue;
        }
        const KEYS: [&str; 10] = [
            "store",
            "source",
            "commit",
            "product",
            "nim",
            "nim-lib",
            "gcc",
            "bwrap",
            "build-dir",
            "out",
        ];
        if !KEYS.contains(&key) || value.is_empty() {
            return Err(format!("unknown or empty option --{key}; {USAGE}"));
        }
        if options.insert(key, value).is_some() {
            return Err(format!("--{key} is repeated"));
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("--{key} is required (no default); {USAGE}"))
    };
    let product = get("product")?;
    if !["hagia", "narthex", "kleis"].contains(&product) {
        return Err(format!(
            "--product must be hagia, narthex or kleis: {product:?}"
        ));
    }
    let commit = get("commit")?;
    if !hex(commit, 40) {
        return Err(format!("--commit must be 40 lowercase hex: {commit:?}"));
    }
    for key in [
        "store",
        "source",
        "nim",
        "nim-lib",
        "gcc",
        "bwrap",
        "build-dir",
        "out",
    ] {
        if !get(key)?.starts_with('/') {
            return Err(format!("--{key} must be absolute"));
        }
    }
    let out = absolute(Path::new(get("out")?))?;
    if out.symlink_metadata().is_ok() {
        return Err(format!("--out already exists: {}", out.display()));
    }
    let build_dir = PathBuf::from(get("build-dir")?);
    crate::package_desktop::private_dir(&build_dir)?;
    let store = std::fs::canonicalize(get("store")?).map_err(|e| format!("--store: {e}"))?;
    let toolchain = probe_toolchain(
        Path::new(get("nim")?),
        Path::new(get("nim-lib")?),
        Path::new(get("gcc")?),
        Path::new(get("bwrap")?),
    )?;
    let tree = crate::bemenu_artifact::signed_tree_under(
        &build_dir,
        Path::new(get("source")?),
        commit,
        &format!("{product}-deps"),
    )?;
    let nimble = tree.tree_dir.join(format!("{product}.nimble"));
    if !std::fs::symlink_metadata(&nimble).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "{product} {commit} has no regular {product}.nimble"
        ));
    }
    let root = nimble_requires(
        &String::from_utf8(read(&nimble)?).map_err(|_| format!("{product}.nimble is not UTF-8"))?,
    )?;
    let manifest = Manifest {
        status: "draft".into(),
        product: product.into(),
        source_commit: commit.into(),
        source_tree: tree.tree.clone(),
        store: store.clone(),
        packages: resolve(&store, &root, &pins, &toolchain.nim_version)?,
        toolchain: toolchain.records,
    };
    let text = manifest.render()?;
    Manifest::parse(&text)?;
    write_new(&out, text.as_bytes())?;
    Ok(vec![format!(
        "nim_deps status=draft product={product} commit={commit} packages={} out={} draft_sha256={} \
         (DRAFT digest, not an authorization: root reviews the file, sets status=reviewed, and \
         supplies the final sha256 independently)",
        manifest.packages.len(),
        out.display(),
        sha256(text.as_bytes())
    )])
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    drop(file);
    set_mode(path, 0o444)
}

// ------------------------------------------------------------------ reviewed

/// A reviewed manifest bound to its independently supplied sha256 and to
/// the exact product source it was reviewed for.
#[derive(Debug, Clone)]
pub struct Reviewed {
    pub manifest: Manifest,
    pub text: String,
    pub sha256: String,
}

/// Load a reviewed manifest: its bytes must hash to `expected` BEFORE they
/// are parsed; its status must be `reviewed`; it must name this product,
/// commit and tree.
pub fn load_reviewed(
    path: &Path,
    expected: &str,
    product: &str,
    commit: &str,
    tree: &str,
) -> Result<Reviewed, String> {
    if !hex(expected, 64) {
        return Err(format!(
            "--nim-deps-sha256 must be 64 lowercase hex: {expected:?}"
        ));
    }
    if !path.is_absolute() || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "--nim-deps must be an absolute regular file: {}",
            path.display()
        ));
    }
    let bytes = read(path)?;
    if sha256(&bytes) != expected {
        return Err(format!(
            "{} is not the expected dependency manifest",
            path.display()
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| "dependency manifest is not UTF-8")?;
    let manifest = Manifest::parse(&text)?;
    if manifest.status != "reviewed" {
        return Err(format!(
            "{} has status {}; only a reviewed manifest may build",
            path.display(),
            manifest.status
        ));
    }
    if manifest.product != product
        || manifest.source_commit != commit
        || manifest.source_tree != tree
    {
        return Err(format!(
            "{} was reviewed for {}@{}, not {product}@{commit}",
            path.display(),
            manifest.product,
            manifest.source_commit
        ));
    }
    Ok(Reviewed {
        manifest,
        text,
        sha256: expected.to_owned(),
    })
}

/// The reviewed closure staged read-only below a private directory; made
/// writable again when dropped so its scratch can be removed.
pub struct Staged {
    pub root: PathBuf,
    pub dirs: Vec<PathBuf>,
}

impl Drop for Staged {
    fn drop(&mut self) {
        writable_again(&self.root);
    }
}

impl Reviewed {
    /// The host toolchain must be exactly the reviewed one.
    pub fn check_toolchain(&self) -> Result<Toolchain, String> {
        let recorded = self.manifest.toolchain_view()?;
        recorded.check_executables()?;
        let path = |role: &str| {
            recorded
                .records
                .iter()
                .find(|r| r.fields.first().is_some_and(|(_, v)| v == role))
                .and_then(|r| r.fields.get(1))
                .map(|(_, p)| PathBuf::from(p))
                .ok_or_else(|| format!("the manifest records no {role}"))
        };
        let probed = probe_toolchain(
            &path("nim")?,
            &path("nim-lib")?,
            &path("gcc")?,
            &path("bwrap")?,
        )?;
        if probed != recorded {
            let expected = recorded
                .records
                .iter()
                .find(|r| !probed.records.contains(r));
            let actual = probed
                .records
                .iter()
                .find(|r| !recorded.records.contains(r));
            return Err(format!(
                "the host toolchain is not the reviewed one: expected {expected:?}; actual {actual:?}; review a new dependency manifest and update its configured digest"
            ));
        }
        Ok(recorded)
    }

    /// Copy every package from the store into `root` (new), verify each
    /// file against the inventory, then make the copy read-only.
    pub fn stage(&self, root: &Path) -> Result<Staged, String> {
        std::fs::create_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let mut dirs = Vec::new();
        for package in &self.manifest.packages {
            let source = self.manifest.store.join(&package.dir);
            let present = inventory(&source, &package.name)?;
            if present != package.files {
                return Err(format!(
                    "store package {} is not the reviewed one",
                    package.dir
                ));
            }
            let dest = root.join(&package.dir);
            for file in &package.files {
                let target = dest.join(&file.path);
                std::fs::create_dir_all(target.parent().ok_or("file without a parent")?)
                    .map_err(|e| e.to_string())?;
                let bytes = read(&source.join(&file.path))?;
                if sha256(&bytes) != file.sha256 {
                    return Err(format!(
                        "{}/{} changed while staging",
                        package.dir, file.path
                    ));
                }
                std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
                set_mode(&target, file.mode & !0o222)?;
            }
            dirs.push(dest);
        }
        read_only_dirs(root)?;
        let staged = Staged {
            root: root.to_path_buf(),
            dirs,
        };
        self.verify(&staged)?;
        Ok(staged)
    }

    /// The staged copy still holds exactly the reviewed files, read-only.
    pub fn verify(&self, staged: &Staged) -> Result<(), String> {
        let mut names = std::fs::read_dir(&staged.root)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        names.sort();
        let mut expected = self
            .manifest
            .packages
            .iter()
            .map(|p| p.dir.clone())
            .collect::<Vec<_>>();
        expected.sort();
        if names != expected {
            return Err("the staged dependency set changed".into());
        }
        for package in &self.manifest.packages {
            let present = inventory(&staged.root.join(&package.dir), &package.name)?;
            let reviewed = package
                .files
                .iter()
                .map(|f| FileEntry {
                    mode: f.mode & !0o222,
                    ..f.clone()
                })
                .collect::<Vec<_>>();
            if present != reviewed {
                return Err(format!("staged package {} changed", package.dir));
            }
        }
        Ok(())
    }
}

fn read_only_dirs(dir: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            read_only_dirs(&path)?;
        }
    }
    set_mode(dir, 0o555)
}

/// Make a read-only staged tree removable again (scratch cleanup).
pub fn writable_again(dir: &Path) {
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.is_dir()
    {
        let _ = set_mode(dir, meta.mode() & 0o7777 | 0o700);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                writable_again(&entry.path());
            }
        }
    }
}
