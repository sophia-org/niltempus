//! The Nim installation a bound build uses: verified, read-only STAGED
//! copies of the reviewed compiler, its installation configuration and its
//! standard library, and a static trace of everything that configuration
//! reads.
//!
//! The installation configuration is kept (the director's ruling): the
//! staged prefix holds `bin/nim`, `config/` and `lib/`, and the compiler
//! finds its configuration beside its own executable, so the build reads
//! the staged `config/nim.cfg` and `config/config.nims` and the staged
//! stdlib (`--lib`), never the live installation, which the builder also
//! hides inside bwrap. User, parent and project configurations and every
//! nimble path stay skipped.
//!
//! Before any build, `trace_config` reads both files as data. It follows
//! every `@include` (which must name a regular file inside the staged
//! configuration) and REFUSES any directive that applies, or cannot be shown
//! not to apply, to this build (Linux, amd64, gcc, release) and names an
//! ambient or unresolved input: an absolute or relative host path, `$HOME`
//! or any other environment expansion, `@putenv`, a compiler or tool path,
//! an implicit import, or, in config.nims, any file, process or environment
//! call or path-bearing switch. `path="$lib/..."` resolves inside the staged
//! stdlib; `nimblepath` entries are recorded as disabled (`--noNimblePath`).
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::bemenu_artifact::set_mode;
use crate::nim_deps::{FileEntry, Toolchain, inventory, inventory_sha256, writable_again};
use crate::records::relative_path;
use crate::{read, sha256};

/// The configuration files the compiler reads from its installation for
/// `nim c` (each may `@include` more).
pub const CONFIG_ROOTS: [&str; 2] = ["nim.cfg", "config.nims"];

/// Symbols known true for this build: Linux, amd64, gcc, `-d:release`.
const TRUE_SYMBOLS: [&str; 6] = ["unix", "linux", "posix", "amd64", "gcc", "release"];
/// Symbols known false for this build (other targets, compilers and modes
/// the builder never selects).
const FALSE_SYMBOLS: [&str; 44] = [
    "false",
    "windows",
    "macosx",
    "osx",
    "ios",
    "bsd",
    "freebsd",
    "netbsd",
    "openbsd",
    "dragonfly",
    "haiku",
    "genode",
    "android",
    "termux",
    "nintendoswitch",
    "emscripten",
    "wasm32",
    "wasm64",
    "mingw",
    "tcc",
    "clang",
    "vcc",
    "icl",
    "icc",
    "vxworks",
    "solaris",
    "aix",
    "arm",
    "arm64",
    "i386",
    "riscv32",
    "riscv64",
    "mips",
    "powerpc64",
    "js",
    "any",
    "standalone",
    "lto",
    "lto_incremental",
    "strip",
    "danger",
    "quick",
    "safety",
    "cpp",
];
const CPUS: [&str; 16] = [
    "i386",
    "amd64",
    "arm",
    "arm64",
    "riscv32",
    "riscv64",
    "mips",
    "mipsel",
    "mips64",
    "mips64el",
    "powerpc",
    "powerpc64",
    "powerpc64el",
    "sparc",
    "wasm32",
    "loongarch64",
];
const OSES: [&str; 17] = [
    "linux",
    "windows",
    "macosx",
    "freebsd",
    "netbsd",
    "openbsd",
    "dragonfly",
    "android",
    "ios",
    "haiku",
    "solaris",
    "aix",
    "any",
    "standalone",
    "nintendoswitch",
    "genode",
    "vxworks",
];
/// C compiler configuration prefixes other than the selected gcc.
const OTHER_COMPILERS: [&str; 11] = [
    "clang",
    "clang_cl",
    "vcc",
    "icl",
    "icc",
    "tcc",
    "llvm_gcc",
    "switch_gcc",
    "env",
    "bcc",
    "zig",
];
/// Calls and names in config.nims that read or write files, run processes,
/// or read the environment or the project location.
const NIMS_DENIED: [&str; 45] = [
    "import",
    "include",
    "from",
    "exec",
    "selfExec",
    "staticExec",
    "gorge",
    "gorgeEx",
    "readFile",
    "writeFile",
    "readLines",
    "fileExists",
    "dirExists",
    "existsFile",
    "existsDir",
    "listFiles",
    "listDirs",
    "rmDir",
    "rmFile",
    "mkDir",
    "mvFile",
    "mvDir",
    "cpFile",
    "cpDir",
    "cd",
    "withDir",
    "thisDir",
    "projectDir",
    "projectPath",
    "projectName",
    "getCurrentDir",
    "putEnv",
    "getEnv",
    "existsEnv",
    "delEnv",
    "patchFile",
    "setCommand",
    "getCommand",
    "paramStr",
    "paramCount",
    "findExe",
    "toExe",
    "toDll",
    "nimcacheDir",
    "requires",
];
/// `switch(...)` / `--x:` options that name paths or inputs.
const PATH_SWITCHES: [&str; 17] = [
    "path",
    "p",
    "lib",
    "nimblepath",
    "clearnimblepath",
    "cincludes",
    "clibdir",
    "passc",
    "passl",
    "import",
    "include",
    "out",
    "o",
    "outdir",
    "nimcache",
    "prefix",
    "clib",
];

/// What the kept configuration reads: every file (relative to the staged
/// prefix, with its sha256) and the notes on inert or disabled directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigTrace {
    pub files: Vec<(String, String)>,
    pub notes: Vec<String>,
}

fn refuse(file: &str, line: usize, text: &str, why: &str) -> String {
    format!("installation config {file}:{line} `{text}` is refused: {why}")
}

/// Three-valued truth of an `@if` condition for this build (None: unknown).
fn eval(condition: &str) -> Option<bool> {
    let spaced = condition.replace('(', " ( ").replace(')', " ) ");
    let tokens = spaced.split_whitespace().collect::<Vec<_>>();
    let mut at = 0;
    let value = or_expr(&tokens, &mut at);
    if at == tokens.len() { value } else { None }
}

fn or_expr(tokens: &[&str], at: &mut usize) -> Option<bool> {
    let mut value = and_expr(tokens, at);
    while tokens.get(*at) == Some(&"or") {
        *at += 1;
        let right = and_expr(tokens, at);
        value = match (value, right) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
    }
    value
}

fn and_expr(tokens: &[&str], at: &mut usize) -> Option<bool> {
    let mut value = not_expr(tokens, at);
    while tokens.get(*at) == Some(&"and") {
        *at += 1;
        let right = not_expr(tokens, at);
        value = match (value, right) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        };
    }
    value
}

fn not_expr(tokens: &[&str], at: &mut usize) -> Option<bool> {
    match tokens.get(*at) {
        Some(&"not") => {
            *at += 1;
            not_expr(tokens, at).map(|v| !v)
        }
        Some(&"(") => {
            *at += 1;
            let value = or_expr(tokens, at);
            if tokens.get(*at) == Some(&")") {
                *at += 1;
                value
            } else {
                *at = tokens.len() + 1;
                None
            }
        }
        Some(symbol) => {
            *at += 1;
            let symbol = symbol.to_ascii_lowercase();
            if TRUE_SYMBOLS.contains(&symbol.as_str()) {
                Some(true)
            } else if FALSE_SYMBOLS.contains(&symbol.as_str()) {
                Some(false)
            } else {
                None
            }
        }
        None => None,
    }
}

/// Strip a `#` comment outside double quotes.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

/// One `@if` frame: whether any branch so far was taken, and this branch.
struct Frame {
    any: Option<bool>,
    current: Option<bool>,
}

fn activity(frames: &[Frame]) -> Option<bool> {
    frames
        .iter()
        .try_fold(true, |all, frame| match frame.current {
            Some(false) => Err(()),
            Some(true) => Ok(all),
            None => Ok(false),
        })
        .map_or(Some(false), |all| all.then_some(true))
}

/// Trace the staged configuration (`prefix/config`) against the staged
/// stdlib (`prefix/lib`).
pub fn trace_config(prefix: &Path) -> Result<ConfigTrace, String> {
    let mut trace = ConfigTrace {
        files: Vec::new(),
        notes: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    trace_cfg(prefix, "config/nim.cfg", &mut trace, &mut seen, 0)?;
    trace_nims(prefix, "config/config.nims", &mut trace)?;
    trace.files.sort();
    Ok(trace)
}

fn staged_file(prefix: &Path, relative: &str) -> Result<Vec<u8>, String> {
    relative_path(relative)?;
    let mut path = prefix.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{relative}: {e}"))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{relative} passes through a link"));
        }
    }
    if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return Err(format!("{relative} is not a regular staged file"));
    }
    read(&path)
}

fn trace_cfg(
    prefix: &Path,
    relative: &str,
    trace: &mut ConfigTrace,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 8 || !seen.insert(relative.to_owned()) {
        return Err(format!(
            "installation config {relative} includes itself or nests too deeply"
        ));
    }
    let bytes = staged_file(prefix, relative)?;
    trace.files.push((relative.to_owned(), sha256(&bytes)));
    let text = String::from_utf8(bytes).map_err(|_| format!("{relative} is not UTF-8"))?;
    let mut frames: Vec<Frame> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = strip_comment(raw).trim();
        if content.is_empty() {
            continue;
        }
        let active = activity(&frames);
        if let Some(directive) = content.strip_prefix('@') {
            let (word, rest) = directive
                .split_once(char::is_whitespace)
                .map_or((directive.trim_end_matches(':'), ""), |(w, r)| {
                    (w, r.trim())
                });
            let rest = rest.trim_end_matches(':').trim();
            match word.to_ascii_lowercase().as_str() {
                "if" => {
                    let value = eval(rest);
                    frames.push(Frame {
                        any: value,
                        current: value,
                    });
                }
                "elif" => {
                    let frame = frames
                        .last_mut()
                        .ok_or_else(|| refuse(relative, line, content, "@elif without @if"))?;
                    let value = eval(rest);
                    frame.current = match (frame.any, value) {
                        (Some(true), _) => Some(false),
                        (Some(false), v) => v,
                        (None, Some(false)) => Some(false),
                        (None, _) => None,
                    };
                    frame.any = match (frame.any, frame.current) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    };
                }
                "else" => {
                    let frame = frames
                        .last_mut()
                        .ok_or_else(|| refuse(relative, line, content, "@else without @if"))?;
                    frame.current = frame.any.map(|any| !any);
                    frame.any = Some(true);
                }
                "end" => {
                    frames
                        .pop()
                        .ok_or_else(|| refuse(relative, line, content, "@end without @if"))?;
                }
                "include" => {
                    if active == Some(false) {
                        continue;
                    }
                    let target = unquote(rest);
                    let base = Path::new(relative)
                        .parent()
                        .and_then(Path::to_str)
                        .unwrap_or("");
                    let joined = format!("{base}/{target}");
                    relative_path(target)
                        .and_then(|()| relative_path(&joined))
                        .and_then(|()| {
                            joined
                                .starts_with("config/")
                                .then_some(())
                                .ok_or_else(|| "outside the staged configuration".to_owned())
                        })
                        .map_err(|why| refuse(relative, line, content, &why))?;
                    trace_cfg(prefix, &joined, trace, seen, depth + 1)?;
                }
                "write" => {}
                _ => {
                    if active != Some(false) {
                        return Err(refuse(
                            relative,
                            line,
                            content,
                            "an environment or unknown directive that applies to this build",
                        ));
                    }
                }
            }
            continue;
        }
        if active == Some(false) {
            continue;
        }
        check_setting(prefix, relative, line, content, active, trace)?;
    }
    if !frames.is_empty() {
        return Err(format!(
            "installation config {relative} has an unterminated @if"
        ));
    }
    Ok(())
}

fn check_setting(
    prefix: &Path,
    file: &str,
    line: usize,
    content: &str,
    active: Option<bool>,
    trace: &mut ConfigTrace,
) -> Result<(), String> {
    let body = content.trim_start_matches('-');
    let split = body
        .find([':', '='])
        .ok_or_else(|| refuse(file, line, content, "not a setting"))?;
    let key = body[..split]
        .trim()
        .trim_end_matches(['%', '&', '^'])
        .trim()
        .to_ascii_lowercase();
    let value = unquote(body[split + 1..].trim_start_matches('='));
    let mut parts = key.split('.').collect::<Vec<_>>();
    if parts.len() > 2 && CPUS.contains(&parts[0]) && OSES.contains(&parts[1]) {
        if (parts[0], parts[1]) != ("amd64", "linux") {
            return Ok(());
        }
        parts = parts.split_off(2);
    }
    if parts.len() > 1 && OTHER_COMPILERS.contains(&parts[0]) {
        return Ok(());
    }
    let base = parts.join(".");
    let undecided = active.is_none();
    match base.as_str() {
        "path" | "p" => {
            let rest = value.strip_prefix("$lib/").ok_or_else(|| {
                refuse(
                    file,
                    line,
                    content,
                    "a search path outside the staged stdlib",
                )
            })?;
            relative_path(rest).map_err(|why| refuse(file, line, content, &why))?;
            let dir = prefix.join("lib").join(rest);
            match std::fs::symlink_metadata(&dir) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => return Err(refuse(file, line, content, "not a staged stdlib directory")),
                Err(_) => trace.notes.push(format!(
                    "{file}:{line} {value}: absent from the staged stdlib (nothing to read)"
                )),
            }
            Ok(())
        }
        "nimblepath" => {
            trace.notes.push(format!(
                "{file}:{line} nimblepath {value}: disabled by --noNimblePath"
            ));
            Ok(())
        }
        _ if PATH_SWITCHES.contains(&base.as_str())
            || base.ends_with(".exe")
            || base.ends_with(".linkerexe")
            || base.ends_with(".path")
            || (base == "cc" && value != "gcc") =>
        {
            Err(refuse(
                file,
                line,
                content,
                "a path, tool or compiler input that applies to this build",
            ))
        }
        _ => {
            let mut plain = value.to_owned();
            while let Some(start) = plain.find("${") {
                let end = plain[start..]
                    .find('}')
                    .map(|e| start + e + 1)
                    .ok_or_else(|| refuse(file, line, content, "an unterminated ${...}"))?;
                plain.replace_range(start..end, "");
            }
            if plain.contains('$') {
                return Err(refuse(file, line, content, "an environment expansion"));
            }
            if plain.contains('/') {
                return Err(refuse(file, line, content, "a host path"));
            }
            if undecided && (value.contains('$') || value.contains('/')) {
                return Err(refuse(
                    file,
                    line,
                    content,
                    "cannot decide whether it applies",
                ));
            }
            Ok(())
        }
    }
}

/// config.nims read as data: refused if it names any file, process or
/// environment call, an import or include, or a path-bearing switch.
fn trace_nims(prefix: &Path, relative: &str, trace: &mut ConfigTrace) -> Result<(), String> {
    let bytes = match staged_file(prefix, relative) {
        Ok(bytes) => bytes,
        Err(_) if std::fs::symlink_metadata(prefix.join(relative)).is_err() => return Ok(()),
        Err(error) => return Err(error),
    };
    trace.files.push((relative.to_owned(), sha256(&bytes)));
    let text = String::from_utf8(bytes).map_err(|_| format!("{relative} is not UTF-8"))?;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = strip_comment(raw).trim();
        let mut words = Vec::new();
        let mut strings = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for c in content.chars() {
            if quoted {
                if c == '"' {
                    strings.push(std::mem::take(&mut current));
                    quoted = false;
                } else {
                    current.push(c);
                }
            } else if c == '"' {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                quoted = true;
            } else if c.is_ascii_alphanumeric() || c == '_' {
                current.push(c);
            } else if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        }
        if quoted {
            return Err(refuse(relative, line, content, "an unterminated string"));
        }
        if !current.is_empty() {
            words.push(current);
        }
        if let Some(word) = words.iter().find(|w| NIMS_DENIED.contains(&w.as_str())) {
            return Err(refuse(
                relative,
                line,
                content,
                &format!("`{word}` reads or writes outside the build's inputs"),
            ));
        }
        if content.starts_with("--") || content.starts_with("switch") {
            let option = if let Some(rest) = content.strip_prefix("--") {
                rest.split([':', ' ']).next().unwrap_or("").to_owned()
            } else {
                strings.first().cloned().unwrap_or_default()
            };
            if PATH_SWITCHES.contains(&option.to_ascii_lowercase().as_str()) {
                return Err(refuse(relative, line, content, "a path-bearing switch"));
            }
        }
        if strings.iter().any(|s| s.contains('/') || s.contains('$')) {
            return Err(refuse(relative, line, content, "a host path or expansion"));
        }
    }
    Ok(())
}

/// The staged, verified, read-only installation.
pub struct Staged {
    pub root: PathBuf,
    pub nim: PathBuf,
    pub lib: PathBuf,
    pub config: PathBuf,
    nim_sha256: String,
    expected: Vec<(PathBuf, Vec<FileEntry>)>,
    pub config_inventory_sha256: String,
    pub stdlib_inventory_sha256: String,
    pub trace: ConfigTrace,
}

impl Drop for Staged {
    fn drop(&mut self) {
        writable_again(&self.root);
    }
}

fn copy_verified(
    from: &Path,
    to: &Path,
    role: &str,
    expected: &str,
) -> Result<Vec<FileEntry>, String> {
    let files = inventory(from, role)?;
    if inventory_sha256(&files)? != expected {
        return Err(format!("the host {role} is not the reviewed one"));
    }
    std::fs::create_dir(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for file in &files {
        let target = to.join(&file.path);
        std::fs::create_dir_all(target.parent().ok_or("file without a parent")?)
            .map_err(|e| e.to_string())?;
        let bytes = read(&from.join(&file.path))?;
        if sha256(&bytes) != file.sha256 {
            return Err(format!("{role} {} changed while staging", file.path));
        }
        std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
        set_mode(&target, file.mode & !0o222)?;
    }
    Ok(files
        .into_iter()
        .map(|f| FileEntry {
            mode: f.mode & !0o222,
            ..f
        })
        .collect())
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

fn record_field<'t>(
    toolchain: &'t Toolchain,
    kind: &str,
    role: &str,
    key: &str,
) -> Result<&'t str, String> {
    toolchain
        .records
        .iter()
        .find(|r| r.kind == kind && r.fields.first().is_some_and(|(_, v)| v == role))
        .and_then(|r| r.fields.iter().find(|(k, _)| k == key))
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| format!("the toolchain records no {kind} {role} {key}"))
}

/// Stage `prefix` (new) from the reviewed toolchain: `bin/nim`, `config/`
/// and `lib/`, each verified against the reviewed records, then read-only;
/// then trace the staged configuration.
pub fn stage(toolchain: &Toolchain, prefix: &Path) -> Result<Staged, String> {
    std::fs::create_dir(prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
    let bin = prefix.join("bin");
    std::fs::create_dir(&bin).map_err(|e| e.to_string())?;
    let nim = bin.join("nim");
    let nim_sha256 = record_field(toolchain, "tool", "nim", "sha256")?.to_owned();
    let bytes = read(&toolchain.tool("nim")?)?;
    if sha256(&bytes) != nim_sha256 {
        return Err("the host nim is not the reviewed one".into());
    }
    std::fs::write(&nim, &bytes).map_err(|e| e.to_string())?;
    set_mode(&nim, 0o555)?;
    let config_inventory_sha256 =
        record_field(toolchain, "tree", "nim-config", "inventory_sha256")?.to_owned();
    let stdlib_inventory_sha256 =
        record_field(toolchain, "tree", "nim-lib", "inventory_sha256")?.to_owned();
    let config = prefix.join("config");
    let lib = prefix.join("lib");
    let expected = vec![
        (
            config.clone(),
            copy_verified(
                &toolchain.tree("nim-config")?,
                &config,
                "nim-config",
                &config_inventory_sha256,
            )?,
        ),
        (
            lib.clone(),
            copy_verified(
                &toolchain.tree("nim-lib")?,
                &lib,
                "nim-lib",
                &stdlib_inventory_sha256,
            )?,
        ),
    ];
    read_only_dirs(prefix)?;
    let mut staged = Staged {
        root: prefix.to_path_buf(),
        nim,
        lib,
        config,
        nim_sha256,
        expected,
        config_inventory_sha256,
        stdlib_inventory_sha256,
        trace: ConfigTrace {
            files: Vec::new(),
            notes: Vec::new(),
        },
    };
    staged.verify()?;
    staged.trace = trace_config(prefix)?;
    Ok(staged)
}

impl Staged {
    /// The staged prefix still holds exactly the reviewed files, read-only.
    pub fn verify(&self) -> Result<(), String> {
        if sha256(&read(&self.nim)?) != self.nim_sha256 {
            return Err("the staged nim changed".into());
        }
        let mut entries = std::fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort();
        if entries != ["bin", "config", "lib"] {
            return Err("the staged nim prefix changed".into());
        }
        for (dir, expected) in &self.expected {
            let role = expected.first().map_or("", |f| f.package.as_str());
            if &inventory(dir, role)? != expected {
                return Err(format!("the staged {} changed", dir.display()));
            }
        }
        Ok(())
    }
}
