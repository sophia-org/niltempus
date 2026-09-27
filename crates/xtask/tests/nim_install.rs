//! The kept Nim installation configuration: it is traced as data against a
//! staged prefix (includes followed only inside the staged configuration),
//! any directive that applies, or cannot be shown not to apply, and reaches
//! an ambient or unresolved input is refused, and the staged prefix is
//! verified, read-only, and refused once it or the reviewed host differs.
//! The ignored control test builds from the staged prefix with the live
//! installation hidden; it needs a real-build slot.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use xtask::nim_deps::{Toolchain, inventory, inventory_sha256};
use xtask::nim_install::{stage, trace_config};
use xtask::records::Record;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, sha256};

/// The shape of the host installation's nim.cfg: every construct it uses,
/// with ambient paths only where they cannot apply to this build.
const HOST_LIKE_CFG: &str = r#"# Configuration file for the Nim Compiler.
#  gcc.path %= "$CC_PATH"
cc = gcc
--parallel_build: "0" # 0 to auto-detect number of processors
@if not nimHasNolineTooLong:
  hint[LineTooLong]=off
@end
threads:on
arm.linux.gcc.exe = "arm-linux-gnueabihf-gcc"
arm64.linux.gcc.linkerexe = "aarch64-linux-gnu-gcc"
path="$lib/deprecated/core"
path="$lib/pure/collections"
path="$lib/windows"
path="$lib/core"
path="$lib/pure"
@if not windows:
  nimblepath="/opt/nimble/pkgs2/"
@else:
  # TODO:
@end
nimblepath="$home/.nimble/pkgs2/"
@if danger or quick:
  obj_checks:off
@end
@if release or danger:
  stacktrace:off
  opt:speed
  define:release
@end
@if unix and mingw:
  amd64.windows.gcc.path = "/usr/bin"
  os = windows
@end
@if unix:
  @if bsd:
    define:useFork
  @elif haiku:
    gcc.options.linker = "-Wl,--as-needed -lnetwork"
  @elif not genode:
    gcc.options.linker = "-ldl"
    clang.options.linker = "-ldl"
  @end
@end
@if freebsd or openbsd or netbsd:
  cincludes: "/usr/local/include"
  clibdir: "/usr/local/lib"
@end
@if vxworks:
  gcc.options.linker %= "-L $WIND_BASE/target/lib/usr/lib/ppc/PPC32/common"
@end
gcc.maxerrorsimpl = "-fmax-errors=3"
@if macosx or freebsd or openbsd:
  cc = clang
@elif windows:
  gcc.options.always %= "-w ${gcc.maxerrorsimpl} -mno-ms-bitfields"
@else:
  gcc.options.always %= "-w ${gcc.maxerrorsimpl}"
@end
gcc.options.always %= "${gcc.options.always} -fno-strict-aliasing"
gcc.options.speed = "-O3 -fno-ident -fno-math-errno"
vcc.exe = "vccexe.exe"
vcc.options.always =  "/nologo"
icl.options.speed = "/Ox /arch:SSE2"
@if lto or lto_incremental:
  @if lto_incremental:
   gcc.options.always %= "${gcc.options.always} -flto=auto"
  @else:
   gcc.options.linker %= "${gcc.options.linker} -flto=auto"
  @end
@end
@if strip:
  gcc.options.linker %= "${gcc.options.linker} -s"
@end
"#;

const HOST_LIKE_NIMS: &str = r#"# this config.nims also needs to exist to prevent future regressions, see #9990
cppDefine "errno"
cppDefine "unix"
when defined(nimStrictMode):
  when defined(nimHasHintAsError):
    switch("hintAsError", "ConvFromXtoItselfNotNeeded")
switch("define", "nimVersion:" & NimVersion) # deadcode
"#;

/// A staged-looking prefix with the given configuration.
fn prefix(dir: &Dir, cfg: &str, nims: Option<&str>) -> PathBuf {
    let prefix = dir.0.join("prefix");
    let _ = fs::remove_dir_all(&prefix);
    for sub in ["config", "lib/pure/collections", "lib/windows", "lib/core"] {
        fs::create_dir_all(prefix.join(sub)).unwrap();
    }
    fs::write(prefix.join("config/nim.cfg"), cfg).unwrap();
    if let Some(nims) = nims {
        fs::write(prefix.join("config/config.nims"), nims).unwrap();
    }
    prefix
}

#[test]
fn the_host_shaped_configuration_is_traced_and_accepted() {
    let dir = Dir::new("nim-install-trace");
    let root = prefix(&dir, HOST_LIKE_CFG, Some(HOST_LIKE_NIMS));
    let trace = trace_config(&root).unwrap();
    let files = trace
        .files
        .iter()
        .map(|(f, _)| f.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, ["config/config.nims", "config/nim.cfg"]);
    assert_eq!(trace.files[1].1, sha256(HOST_LIKE_CFG.as_bytes()));
    let notes = trace.notes.join("\n");
    assert!(
        notes.contains("$lib/deprecated/core: absent from the staged stdlib"),
        "{notes}"
    );
    assert!(
        notes.contains("nimblepath /opt/nimble/pkgs2/: disabled by --noNimblePath"),
        "{notes}"
    );
    assert!(
        notes.contains("nimblepath $home/.nimble/pkgs2/: disabled"),
        "{notes}"
    );
    // No config.nims is fine; an included file inside the staged
    // configuration is traced.
    let root = prefix(&dir, "@include \"extra.cfg\"\ncc = gcc\n", None);
    fs::write(root.join("config/extra.cfg"), "threads:on\n").unwrap();
    let trace = trace_config(&root).unwrap();
    let files = trace
        .files
        .iter()
        .map(|(f, _)| f.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, ["config/extra.cfg", "config/nim.cfg"]);
}

#[test]
fn ambient_or_unresolved_references_are_refused() {
    let dir = Dir::new("nim-install-refuse");
    for (cfg, expected) in [
        ("path=\"/usr/local/lib/nim\"\n", "outside the staged stdlib"),
        ("path=\"$home/.nim\"\n", "outside the staged stdlib"),
        ("path=\"$lib/../../etc\"\n", "escapes"),
        (
            "gcc.options.always = \"-I/usr/local/include\"\n",
            "a host path",
        ),
        (
            "gcc.options.linker %= \"$LDFLAGS\"\n",
            "an environment expansion",
        ),
        (
            "@putenv \"CC\" \"cc\"\n",
            "environment or unknown directive",
        ),
        (
            "gcc.exe = \"/opt/gcc/bin/gcc\"\n",
            "a path, tool or compiler input",
        ),
        (
            "amd64.linux.gcc.path = \"/opt/gcc\"\n",
            "a path, tool or compiler input",
        ),
        (
            "cincludes: \"/usr/local/include\"\n",
            "a path, tool or compiler input",
        ),
        ("cc = clang\n", "a path, tool or compiler input"),
        ("import: \"std/os\"\n", "a path, tool or compiler input"),
        (
            "@if someUndecidable:\n  gcc.options.always = \"-I/x\"\n@end\n",
            "a host path",
        ),
        (
            "@if someUndecidable:\n  clibdir: \"/x\"\n@end\n",
            "a path, tool or compiler input",
        ),
        ("@include \"../../etc/extra.cfg\"\n", "escapes"),
        ("@include \"/etc/extra.cfg\"\n", "escapes"),
        ("@include \"missing.cfg\"\n", "missing.cfg"),
        ("@if unix:\n  threads:on\n", "unterminated @if"),
        ("@end\n", "@end without @if"),
    ] {
        let root = prefix(&dir, cfg, None);
        let error = trace_config(&root).unwrap_err();
        assert!(error.contains(expected), "{cfg:?}: {error}");
    }
    // An include cycle, and an included file that reaches out.
    let root = prefix(&dir, "@include \"a.cfg\"\n", None);
    fs::write(root.join("config/a.cfg"), "@include \"nim.cfg\"\n").unwrap();
    assert!(trace_config(&root).unwrap_err().contains("includes itself"));
    let root = prefix(&dir, "@include \"a.cfg\"\n", None);
    fs::write(root.join("config/a.cfg"), "path=\"/srv/nim\"\n").unwrap();
    assert!(trace_config(&root).unwrap_err().contains("config/a.cfg:1"));
    // A linked include is not a staged file.
    let root = prefix(&dir, "@include \"a.cfg\"\n", None);
    std::os::unix::fs::symlink("/etc/hostname", root.join("config/a.cfg")).unwrap();
    assert!(trace_config(&root).is_err());
    for (nims, expected) in [
        ("import std/os\n", "`import`"),
        ("exec \"true\"\n", "`exec`"),
        ("putEnv(\"CC\", \"cc\")\n", "`putEnv`"),
        ("let x = readFile(\"f\")\n", "`readFile`"),
        ("switch(\"path\", \"src\")\n", "a path-bearing switch"),
        ("--path:src\n", "a path-bearing switch"),
        ("cppDefine \"/usr/x\"\n", "a host path or expansion"),
        (
            "switch(\"define\", \"$HOME\")\n",
            "a host path or expansion",
        ),
    ] {
        let root = prefix(&dir, "cc = gcc\n", Some(nims));
        let error = trace_config(&root).unwrap_err();
        assert!(error.contains(expected), "{nims:?}: {error}");
    }
}

/// A fake reviewed host installation and the toolchain records naming it.
fn fake_host(dir: &Dir) -> (PathBuf, Toolchain) {
    let host = dir.0.join("host");
    for sub in ["bin", "config", "lib/pure", "lib/core"] {
        fs::create_dir_all(host.join(sub)).unwrap();
    }
    fs::write(host.join("bin/nim"), "#!/bin/sh\n").unwrap();
    fs::write(
        host.join("config/nim.cfg"),
        "cc = gcc\npath=\"$lib/pure\"\n",
    )
    .unwrap();
    fs::write(host.join("config/config.nims"), HOST_LIKE_NIMS).unwrap();
    fs::write(host.join("lib/system.nim"), "# system\n").unwrap();
    fs::write(host.join("lib/pure/os.nim"), "# os\n").unwrap();
    let tree = |role: &str, path: PathBuf| {
        let files = inventory(&path, role).unwrap();
        Record::of("tree")
            .with("role", role)
            .with("path", path.display().to_string())
            .with("files", files.len().to_string())
            .with("inventory_sha256", inventory_sha256(&files).unwrap())
    };
    let nim = host.join("bin/nim");
    let toolchain = Toolchain {
        records: vec![
            Record::of("tool")
                .with("role", "nim")
                .with("path", nim.display().to_string())
                .with("resolved", nim.display().to_string())
                .with("version", "Nim Compiler Version 2.2.12 [Linux: amd64]")
                .with("sha256", sha256(&fs::read(&nim).unwrap())),
            tree("nim-config", host.join("config")),
            tree("nim-lib", host.join("lib")),
        ],
        nim_version: vec![2, 2, 12],
    };
    (host, toolchain)
}

#[test]
fn the_staged_prefix_is_verified_read_only_and_refused_once_changed() {
    let dir = Dir::new("nim-install-stage");
    let (host, toolchain) = fake_host(&dir);
    let staged = stage(&toolchain, &dir.0.join("prefix")).unwrap();
    assert_eq!(staged.nim, dir.0.join("prefix/bin/nim"));
    assert_eq!(staged.lib, dir.0.join("prefix/lib"));
    for file in inventory(&staged.root, "prefix").unwrap() {
        assert_eq!(file.mode & 0o222, 0, "{}", file.path);
    }
    assert_eq!(
        fs::metadata(&staged.config).unwrap().permissions().mode() & 0o777,
        0o555
    );
    let files = staged
        .trace
        .files
        .iter()
        .map(|(f, _)| f.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, ["config/config.nims", "config/nim.cfg"]);
    staged.verify().unwrap();
    // A staged copy changed after staging.
    let os = staged.lib.join("pure/os.nim");
    fs::set_permissions(staged.lib.join("pure"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&os, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&os, "# changed\n").unwrap();
    assert!(staged.verify().is_err());
    drop(staged);
    // The host changed after review: nothing is staged from it.
    fs::write(host.join("config/nim.cfg"), "cc = gcc\n").unwrap();
    let error = stage(&toolchain, &dir.0.join("prefix-again"))
        .err()
        .unwrap();
    assert!(error.contains("not the reviewed one"), "{error}");
    fs::write(
        host.join("config/nim.cfg"),
        "cc = gcc\npath=\"$lib/pure\"\n",
    )
    .unwrap();
    // A reviewed configuration that reaches out is refused at staging.
    let dir = Dir::new("nim-install-stage-refuse");
    let (host, mut toolchain) = fake_host(&dir);
    fs::write(host.join("config/nim.cfg"), "path=\"/srv/nim\"\n").unwrap();
    let files = inventory(&host.join("config"), "nim-config").unwrap();
    toolchain.records[1] = Record::of("tree")
        .with("role", "nim-config")
        .with("path", host.join("config").display().to_string())
        .with("files", files.len().to_string())
        .with("inventory_sha256", inventory_sha256(&files).unwrap());
    let error = stage(&toolchain, &dir.0.join("prefix")).err().unwrap();
    assert!(error.contains("outside the staged stdlib"), "{error}");
}

/// Real-build control (needs a slot): a staged prefix of the host's nim
/// compiles a program with the live installation hidden inside bwrap, and
/// the same build pointed at the live stdlib fails. Inputs are explicit:
/// SOPHIA_TEST_NIM, SOPHIA_TEST_NIM_LIB, SOPHIA_TEST_GCC, SOPHIA_TEST_BWRAP.
#[test]
#[ignore = "real Nim build: run in a granted slot with the explicit toolchain"]
fn a_build_reads_only_the_staged_installation() {
    let var = |name: &str| {
        let value = std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"));
        assert!(value.starts_with('/'), "{name} must be absolute");
        PathBuf::from(value)
    };
    let toolchain = xtask::nim_deps::probe_toolchain(
        &var("SOPHIA_TEST_NIM"),
        &var("SOPHIA_TEST_NIM_LIB"),
        &var("SOPHIA_TEST_GCC"),
        &var("SOPHIA_TEST_BWRAP"),
    )
    .unwrap();
    let dir = Dir::new("nim-install-control");
    let scratch = dir.0.join("scratch");
    let (tree, home, deps, out) = (
        scratch.join("tree"),
        scratch.join("home"),
        scratch.join("deps"),
        scratch.join("out"),
    );
    for path in [&tree.join("src"), &home, &deps, &out] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(
        tree.join("src/control.nim"),
        "import std/os\necho \"staged-nim-control \", NimVersion, \" \", paramCount()\n",
    )
    .unwrap();
    let staged = stage(&toolchain, &scratch.join("nim")).unwrap();
    let binary = out.join("control");
    let paths = xtask::product_artifact::NimPaths {
        scratch: &scratch,
        home: &home,
        tree: &tree,
        deps: &deps,
        dep_dirs: &[],
        binary: &binary,
    };
    let (mut command, argv) =
        xtask::product_artifact::nim_command(&toolchain, &staged, &paths, "src/control.nim")
            .unwrap();
    assert_eq!(argv[0], staged.nim.display().to_string());
    xtask::bemenu_artifact::bounded(
        &mut command,
        std::time::Duration::from_secs(900),
        "control build",
    )
    .unwrap();
    let output = std::process::Command::new(&binary).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("staged-nim-control "));
    staged.verify().unwrap();
    // Negative control: the same build pointed at the LIVE stdlib, which the
    // sandbox hides, fails.
    let live = toolchain.tree("nim-lib").unwrap();
    let live_argv = argv
        .iter()
        .map(|a| {
            if a.starts_with("--lib:") {
                format!("--lib:{}", live.display())
            } else if a.starts_with("-o:") {
                format!("-o:{}", out.join("live").display())
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>();
    let mut live_command =
        xtask::product_artifact::sandboxed(&toolchain, &staged.root, &paths, &live_argv).unwrap();
    assert!(
        xtask::bemenu_artifact::bounded(
            &mut live_command,
            std::time::Duration::from_secs(900),
            "live control"
        )
        .is_err()
    );
    assert!(!out.join("live").exists());
}
