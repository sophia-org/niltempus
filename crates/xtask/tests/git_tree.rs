//! `git_tree::inventory` hashes with Git itself, so its identity must not
//! depend on the caller's working directory. Run from outside any
//! repository, a tree holding `.gitattributes` and `.gitmodules` entries must
//! produce the tree ID Git records when it indexes the same files. Sophia's
//! staged tree carries `.gitattributes` at its root and in its vendored SDKs.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xtask-git-tree-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Four regular files, one executable, and both entry names Git's fsck
/// treats specially. Neither attribute changes how Git stores these bytes.
fn fixture(root: &Path) {
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a.txt"), "a\n").unwrap();
    fs::write(root.join(".gitattributes"), "*.bin binary\n").unwrap();
    fs::write(
        root.join("sub/.gitmodules"),
        "[submodule \"x\"]\n\tpath = x\n\turl = ../x\n",
    )
    .unwrap();
    fs::write(root.join("sub/run.sh"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("sub/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
}

/// Git's own identity for the same files: index them into a private
/// repository and write the tree. Independent of `git hash-object`.
fn indexed_tree(scratch: &Path, root: &Path) -> String {
    let git_dir = scratch.join("index.git");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg(format!("--git-dir={}", git_dir.display()))
            .arg(format!("--work-tree={}", root.display()))
            .args(["-c", "core.fileMode=true", "-c", "core.autocrlf=false"])
            .args(args)
            // Root the "." pathspec in the fixture, not the harness's cwd.
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let init = Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&git_dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap();
    assert!(init.success());
    git(&["add", "-A", "--", "."]);
    git(&["write-tree"])
}

/// Child half: run by the test below with a non-repository working
/// directory, so the parent's working directory is never changed.
#[test]
#[ignore = "child process of inventory_outside_a_repository_matches_the_indexed_tree"]
fn inventory_child() {
    let root = std::env::var_os("XTASK_GIT_TREE_FIXTURE").expect("fixture root");
    let inventory = xtask::git_tree::inventory(Path::new(&root)).unwrap();
    println!(
        "git_tree_inventory tree={} files={}",
        inventory.tree,
        inventory.files.len()
    );
}

#[test]
fn inventory_outside_a_repository_matches_the_indexed_tree() {
    let dir = Dir::new("outside");
    let root = dir.0.join("tree");
    let outside = dir.0.join("outside");
    fixture(&root);
    fs::create_dir(&outside).unwrap();
    // Discovery stops at the scratch directory: no repository is in scope.
    let ceiling = dir.0.display().to_string();
    let probe = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(&outside)
        .env("GIT_CEILING_DIRECTORIES", &ceiling)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        !probe.status.success(),
        "the child cwd is inside a repository"
    );

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inventory_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&outside)
        .env("XTASK_GIT_TREE_FIXTURE", &root)
        .env("GIT_CEILING_DIRECTORIES", &ceiling)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "inventory outside a repository failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = stdout
        .lines()
        // libtest prints it after "test inventory_child ... " on one line.
        .find_map(|line| line.split_once("git_tree_inventory ").map(|(_, rest)| rest))
        .expect("child inventory record");
    let expected = indexed_tree(&dir.0, &root);
    assert_eq!(line, format!("tree={expected} files=4"));
}
