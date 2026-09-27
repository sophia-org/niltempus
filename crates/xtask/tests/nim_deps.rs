//! The Nim dependency closure: the record encoding is canonical, `.nimble`
//! requirements are read as data (unsupported forms fail by name), the
//! resolver uses explicit pins only, a manifest is refused unless canonical
//! and reviewed under its independently supplied sha256, and the staged
//! closure is read-only and verified. Nothing here runs nim or nimble.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use xtask::nim_deps::{
    Manifest, inventory, load_reviewed, nimble_requires, parse_requirement, resolve,
};
use xtask::product_artifact::nim_flags;
use xtask::records::{Record, parse_line, parse_text, relative_path};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, sha256};

fn checksum(tag: u8) -> String {
    format!("{tag:02x}").repeat(20)
}

/// A nimble-installed package directory: its files, `NAME.nimble` with the
/// given requires lines, and a nimblemeta.json listing exactly those files.
fn package(store: &Path, name: &str, version: &str, tag: u8, requires: &[&str]) -> String {
    let dir = format!("{name}-{version}-{}", checksum(tag));
    let root = store.join(&dir);
    fs::create_dir_all(root.join(name)).unwrap();
    let nimble = format!(
        "version = \"{version}\"\nsrcDir = \"src\"\n\n{}\n",
        requires
            .iter()
            .map(|r| format!("requires \"{r}\""))
            .collect::<Vec<_>>()
            .join("\n")
    );
    fs::write(root.join(format!("{name}.nimble")), nimble).unwrap();
    fs::write(root.join(format!("{name}.nim")), format!("# {name}\n")).unwrap();
    fs::write(root.join(name).join("inner.nim"), "discard\n").unwrap();
    let meta = serde_json::json!({
        "version": 1,
        "metaData": {
            "url": format!("https://example.invalid/{name}"),
            "downloadMethod": "git",
            "vcsRevision": "0".repeat(40),
            "files": [
                format!("/{name}.nimble"),
                format!("/{name}.nim"),
                format!("/{name}/inner.nim"),
            ],
            "binaries": [],
            "specialVersions": [version],
        }
    });
    fs::write(root.join("nimblemeta.json"), meta.to_string()).unwrap();
    dir
}

fn pins(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

const NIM: [u64; 3] = [2, 2, 12];

#[test]
fn records_have_exactly_one_spelling() {
    let record = Record::of("tool")
        .with("role", "gcc")
        .with("version", "gcc (GCC) 14.2.1 \"x\" \\ y")
        .with("empty", "");
    let line = record.encode().unwrap();
    assert_eq!(
        line,
        r#"tool role=gcc version="gcc (GCC) 14.2.1 \"x\" \\ y" empty="""#
    );
    assert_eq!(parse_line(&line).unwrap(), record);
    // A bare-able value quoted, a doubled space, a trailing space, an
    // unknown escape, a missing '=' and an unterminated quote: all refused.
    for bad in [
        r#"tool role="gcc""#,
        "tool  role=gcc",
        "tool role=gcc ",
        r#"tool role="a\nb""#,
        "tool role",
        r#"tool role="gcc"#,
        "Tool role=gcc",
        "tool Role=gcc",
    ] {
        assert!(parse_line(bad).is_err(), "{bad:?}");
    }
    assert!(Record::of("note").with("text", "a\tb").encode().is_err());
    assert!(parse_text("tool role=gcc").is_err(), "no final newline");
    assert!(parse_text("tool role=gcc\n\n").is_err(), "empty line");
    for bad in ["", "/abs", "a/../b", "..", "a//b", "./a", "a/", "a\nb"] {
        assert!(relative_path(bad).is_err(), "{bad:?}");
    }
    relative_path("examples/config/default.kdl").unwrap();
}

#[test]
fn requirements_are_read_as_data_and_unsupported_forms_fail_by_name() {
    let text = "version = \"1.0\"\nrequires \"nim >= 2.0.0\" # comment\nrequires \"a >= 1.0 & < 2\",\n         \"b\",\n         \"c == 0.12.0\"\n\ntask t, \"Run (requires cargo)\":\n  exec \"x\"\n";
    let parsed = nimble_requires(text).unwrap();
    let names = parsed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["nim", "a", "b", "c"]);
    assert!(parsed[1].allows(&[1, 5]));
    assert!(!parsed[1].allows(&[2]));
    assert!(parsed[3].allows(&[0, 12, 0]) && !parsed[3].allows(&[0, 12, 1]));
    for (text, named) in [
        ("requires \"a ^= 1.0\"\n", "a ^= 1.0"),
        ("requires \"a ~= 1.0\"\n", "a ~= 1.0"),
        ("requires \"a#head\"\n", "a#head"),
        (
            "requires \"https://example.invalid/a\"\n",
            "https://example.invalid/a",
        ),
        ("requires \"a >= x\"\n", "a >= x"),
        ("when defined(linux):\n  requires \"a\"\n", "requires \"a\""),
        ("requires(\"a\")\n", "(\"a\")"),
        ("requires someVariable\n", "someVariable"),
        ("include other\n", "include other"),
    ] {
        let error = nimble_requires(text).unwrap_err();
        assert!(
            error.contains("unsupported requires expression") && error.contains(named),
            "{text:?}: {error}"
        );
    }
    assert!(parse_requirement("a >= 1.2.3").is_ok());
}

#[test]
fn resolution_takes_explicit_pins_only() {
    let dir = Dir::new("nim-deps-resolve");
    let store = dir.0.join("store");
    fs::create_dir(&store).unwrap();
    package(&store, "a", "1.0.0", 1, &["nim >= 2.0.0", "b >= 1.0"]);
    package(&store, "b", "1.0.0", 2, &["nim >= 1.6"]);
    package(&store, "b", "2.0.0", 3, &[]);
    let root = nimble_requires("requires \"nim >= 2.2.4\"\nrequires \"a\"\n").unwrap();

    let closure = resolve(
        &store,
        &root,
        &pins(&[("a", "1.0.0"), ("b", "2.0.0")]),
        &NIM,
    )
    .unwrap();
    let chosen = closure
        .iter()
        .map(|p| format!("{}@{}", p.name, p.version))
        .collect::<Vec<_>>();
    assert_eq!(chosen, ["a@1.0.0", "b@2.0.0"]);
    assert_eq!(closure[0].checksum, checksum(1));
    assert_eq!(closure[0].requires, "nim >= 2.0.0, b >= 1.0");

    // A transitive requirement without a pin: refused, naming the candidates.
    let error = resolve(&store, &root, &pins(&[("a", "1.0.0")]), &NIM).unwrap_err();
    assert!(
        error.contains("not pinned") && error.contains("b-1.0.0-") && error.contains("b-2.0.0-"),
        "{error}"
    );
    // A pin that violates a constraint.
    package(&store, "b", "0.5.0", 4, &[]);
    let error = resolve(
        &store,
        &root,
        &pins(&[("a", "1.0.0"), ("b", "0.5.0")]),
        &NIM,
    )
    .unwrap_err();
    assert!(error.contains("does not satisfy"), "{error}");
    // A pin outside the closure.
    let error = resolve(
        &store,
        &root,
        &pins(&[("a", "1.0.0"), ("b", "2.0.0"), ("z", "1.0")]),
        &NIM,
    )
    .unwrap_err();
    assert!(error.contains("pins not in the closure"), "{error}");
    // The recorded nim is too old for a requirement.
    assert!(
        resolve(
            &store,
            &root,
            &pins(&[("a", "1.0.0"), ("b", "2.0.0")]),
            &[2, 0, 8]
        )
        .is_err()
    );
    // Two store directories for one pin: never a pick.
    package(&store, "b", "2.0.0", 5, &[]);
    let error = resolve(
        &store,
        &root,
        &pins(&[("a", "1.0.0"), ("b", "2.0.0")]),
        &NIM,
    )
    .unwrap_err();
    assert!(error.contains("matches 2 store directories"), "{error}");
}

#[test]
fn a_package_must_hold_exactly_its_recorded_files() {
    let dir = Dir::new("nim-deps-meta");
    let store = dir.0.join("store");
    fs::create_dir(&store).unwrap();
    let a = package(&store, "a", "1.0.0", 1, &[]);
    let root = nimble_requires("requires \"a\"\n").unwrap();
    fs::write(store.join(&a).join("stray.nim"), "discard\n").unwrap();
    let error = resolve(&store, &root, &pins(&[("a", "1.0.0")]), &NIM).unwrap_err();
    assert!(error.contains("stray.nim"), "{error}");
    fs::remove_file(store.join(&a).join("stray.nim")).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", store.join(&a).join("link.nim")).unwrap();
    assert!(resolve(&store, &root, &pins(&[("a", "1.0.0")]), &NIM).is_err());
}

/// A reviewed manifest over a real fake store, with a stand-in toolchain.
fn manifest(store: &Path, status: &str) -> Manifest {
    let root = nimble_requires("requires \"a\"\n").unwrap();
    Manifest {
        status: status.into(),
        product: "hagia".into(),
        source_commit: "a".repeat(40),
        source_tree: "b".repeat(40),
        store: store.to_path_buf(),
        toolchain: vec![
            Record::of("tool")
                .with("role", "nim")
                .with("path", "/usr/bin/nim")
                .with("resolved", "/usr/lib/nim/bin/nim")
                .with("version", "Nim Compiler Version 2.2.12 [Linux: amd64]")
                .with("sha256", "c".repeat(64)),
        ],
        packages: resolve(store, &root, &pins(&[("a", "1.0.0"), ("b", "1.0.0")]), &NIM).unwrap(),
    }
}

fn fake_store(dir: &Dir) -> PathBuf {
    let store = dir.0.join("store");
    fs::create_dir(&store).unwrap();
    package(&store, "a", "1.0.0", 1, &["b"]);
    package(&store, "b", "1.0.0", 2, &[]);
    store
}

#[test]
fn a_manifest_is_accepted_only_in_canonical_form() {
    let dir = Dir::new("nim-deps-manifest");
    let store = fake_store(&dir);
    let text = manifest(&store, "reviewed").render().unwrap();
    assert_eq!(
        Manifest::parse(&text).unwrap(),
        manifest(&store, "reviewed")
    );
    assert!(text.contains("note text=\"host toolchain identity is recorded and re-checked; it is not a fully reproducible closure\"\n"));
    let lines = text.lines().collect::<Vec<_>>();
    let swap = |i: usize, j: usize| {
        let mut l = lines.clone();
        l.swap(i, j);
        l.join("\n") + "\n"
    };
    let package_lines = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("package "))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let file_lines = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("file "))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    for (label, bad) in [
        (
            "packages out of order",
            swap(package_lines[0], package_lines[1]),
        ),
        ("files out of order", swap(file_lines[0], file_lines[1])),
        ("duplicate file", text.clone() + lines[file_lines[0]] + "\n"),
        (
            "unknown status",
            text.replacen("status=reviewed", "status=approved", 1),
        ),
        ("schema", text.replacen("schema=1", "schema=2", 1)),
        (
            "no note",
            text.replacen(lines[3], "", 1).replacen("\n\n", "\n", 1),
        ),
        (
            "escaping path",
            text.replacen("path=a.nim ", "path=../a.nim ", 1),
        ),
        (
            "absolute path",
            text.replacen("path=a.nim ", "path=/a.nim ", 1),
        ),
        (
            "file content changed",
            text.replacen(&sha256(b"# a\n"), &"0".repeat(64), 1),
        ),
    ] {
        assert!(Manifest::parse(&bad).is_err(), "{label}");
    }
}

#[test]
fn only_a_reviewed_manifest_under_its_supplied_digest_builds() {
    let dir = Dir::new("nim-deps-review");
    let store = fake_store(&dir);
    let write = |name: &str, text: &str| {
        let path = dir.0.join(name);
        fs::write(&path, text).unwrap();
        (path, sha256(text.as_bytes()))
    };
    let (draft, draft_sha) = write("draft", &manifest(&store, "draft").render().unwrap());
    let (reviewed, sha) = write("reviewed", &manifest(&store, "reviewed").render().unwrap());
    let (commit, tree) = ("a".repeat(40), "b".repeat(40));
    let error = load_reviewed(&draft, &draft_sha, "hagia", &commit, &tree).unwrap_err();
    assert!(error.contains("only a reviewed manifest"), "{error}");
    assert!(load_reviewed(&reviewed, &draft_sha, "hagia", &commit, &tree).is_err());
    assert!(load_reviewed(&reviewed, "short", "hagia", &commit, &tree).is_err());
    assert!(load_reviewed(&reviewed, &sha, "narthex", &commit, &tree).is_err());
    assert!(load_reviewed(&reviewed, &sha, "hagia", &"c".repeat(40), &tree).is_err());
    assert!(load_reviewed(&reviewed, &sha, "hagia", &commit, &"c".repeat(40)).is_err());
    assert!(load_reviewed(Path::new("reviewed"), &sha, "hagia", &commit, &tree).is_err());
    let loaded = load_reviewed(&reviewed, &sha, "hagia", &commit, &tree).unwrap();
    assert_eq!(loaded.sha256, sha);

    // Staged read-only, verified, and refused once anything changes.
    let staged = loaded.stage(&dir.0.join("staged")).unwrap();
    assert_eq!(staged.dirs.len(), 2);
    for file in inventory(&staged.dirs[0], "a").unwrap() {
        assert_eq!(file.mode & 0o222, 0, "{}", file.path);
    }
    assert_eq!(
        fs::metadata(&staged.dirs[0]).unwrap().permissions().mode() & 0o777,
        0o555
    );
    loaded.verify(&staged).unwrap();
    let inner = staged.dirs[1].join("b.nim");
    fs::set_permissions(&staged.dirs[1], fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&inner, "# changed\n").unwrap();
    assert!(loaded.verify(&staged).is_err());
    drop(staged);

    // A store package changed after review is refused before staging.
    fs::write(
        store.join(format!("b-1.0.0-{}", checksum(2))).join("b.nim"),
        "# b2\n",
    )
    .unwrap();
    assert!(loaded.stage(&dir.0.join("staged-again")).is_err());
}

#[test]
fn the_compiler_sees_only_explicit_paths() {
    let deps = [PathBuf::from("/b/deps/a-1"), PathBuf::from("/b/deps/b-1")];
    let lib = Path::new("/b/nim/lib");
    let gcc = Path::new("/usr/bin/gcc");
    let flags = nim_flags(lib, gcc, &deps);
    for flag in [
        "--noNimblePath",
        "--clearNimblePath",
        "--skipUserCfg:on",
        "--skipParentCfg:on",
        "--skipProjCfg:on",
        "--lib:/b/nim/lib",
        "--gcc.exe:/usr/bin/gcc",
        "--path:/b/deps/a-1",
        "--path:/b/deps/b-1",
    ] {
        assert!(flags.iter().any(|f| f == flag), "{flag}");
    }
    // The installation configuration is kept (one approach): it is staged,
    // traced and recorded, never skipped.
    assert!(!flags.iter().any(|f| f.starts_with("--skipCfg")));
}
