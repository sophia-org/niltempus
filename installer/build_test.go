package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func sourceFixture(t *testing.T) Source {
	t.Helper()
	root := filepath.Join(t.TempDir(), "source with ' spaces")
	if err := os.Mkdir(root, 0700); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"init", "--quiet"}, {"config", "user.name", "Installer test fixture"}, {"config", "user.email", "installer@example.invalid"}, {"config", "commit.gpgsign", "false"}} {
		if _, err := git(root, args...); err != nil {
			t.Fatal(err)
		}
	}
	for name, data := range map[string]string{"source": "first", "unchanged": "same", ".gitignore": "target/\n"} {
		if err := writeFile(filepath.Join(root, name), []byte(data), 0644); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := git(root, "add", "."); err != nil {
		t.Fatal(err)
	}
	if _, err := git(root, "commit", "--quiet", "-m", "unsigned offline fixture"); err != nil {
		t.Fatal(err)
	}
	head, err := git(root, "rev-parse", "HEAD")
	if err != nil {
		t.Fatal(err)
	}
	return Source{Repository: Repository{Path: root}, Commit: head, Signature: "N"}
}

func TestBuildCheckoutPreservesUnchangedInputsAcrossCommits(t *testing.T) {
	source := sourceFixture(t)
	cache := t.TempDir()
	root, err := buildSource("sophia", source, cache, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, "unchanged")
	oldTime := time.Unix(1000000000, 0)
	if err := os.Chtimes(path, oldTime, oldTime); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	target := t.TempDir()
	if err := linkBuildTarget(root, target); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(target, "compiler-output"), []byte("cached"), 0644); err != nil {
		t.Fatal(err)
	}
	for _, advance := range []bool{false, true} {
		if advance {
			if err := writeFile(filepath.Join(source.Path, "source"), []byte("second"), 0644); err != nil {
				t.Fatal(err)
			}
			if _, err := git(source.Path, "commit", "--quiet", "-am", "next input"); err != nil {
				t.Fatal(err)
			}
			source.Commit, err = git(source.Path, "rev-parse", "HEAD")
			if err != nil {
				t.Fatal(err)
			}
			// Work in the original checkout is never copied into the build.
			if err := writeFile(filepath.Join(source.Path, "source"), []byte("uncommitted"), 0644); err != nil {
				t.Fatal(err)
			}
		}
		again, err := buildSource("sophia", source, cache, t.TempDir())
		if err != nil || again != root {
			t.Fatalf("cache not reused: %s: %v", again, err)
		}
		after, err := os.Stat(path)
		if err != nil || !os.SameFile(before, after) || !after.ModTime().Equal(before.ModTime()) {
			t.Fatalf("unchanged input was rewritten: %v", err)
		}
		if err := linkBuildTarget(root, target); err != nil {
			t.Fatal(err)
		}
	}
	if data, err := os.ReadFile(filepath.Join(root, "source")); err != nil || string(data) != "second" {
		t.Fatalf("new committed input not selected: %q: %v", data, err)
	}
	if data, err := os.ReadFile(filepath.Join(source.Path, "source")); err != nil || string(data) != "uncommitted" {
		t.Fatal("changed original checkout")
	}
	if err := writeFile(filepath.Join(root, "source"), []byte("unexpected cache edit"), 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := buildSource("sophia", source, cache, t.TempDir()); err == nil {
		t.Fatal("accepted dirty build input")
	}
	if data, _ := os.ReadFile(filepath.Join(root, "source")); string(data) != "unexpected cache edit" {
		t.Fatal("discarded cache edits")
	}
}

func TestBuildCheckoutSeparatesOriginsAndRejectsWrongTarget(t *testing.T) {
	cache := t.TempDir()
	first, err := buildSource("sophia", sourceFixture(t), cache, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	second, err := buildSource("sophia", sourceFixture(t), cache, t.TempDir())
	if err != nil || first == second {
		t.Fatalf("different origins share a checkout: %v", err)
	}
	if err := linkBuildTarget(first, t.TempDir()); err != nil {
		t.Fatal(err)
	}
	if err := linkBuildTarget(first, t.TempDir()); err == nil {
		t.Fatal("silently redirected compiler outputs")
	}
}

func TestBuildInputsAreSafeUnderPermissiveUmask(t *testing.T) {
	if os.Getenv("DESKTOP_UMASK_TEST_CHILD") != "1" {
		cmd := exec.Command("sh", "-c", `umask 0002; exec "$@"`, "umask-test", os.Args[0], "-test.run=^TestBuildInputsAreSafeUnderPermissiveUmask$")
		cmd.Env = append(os.Environ(), "DESKTOP_UMASK_TEST_CHILD=1")
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("permissive-umask child: %v: %s", err, out)
		}
		return
	}
	source := sourceFixture(t)
	// This reproduces the rejected profile's exact filesystem mode.
	if err := os.Chmod(filepath.Join(source.Path, "source"), 0664); err != nil {
		t.Fatal(err)
	}
	root, err := buildSource("hagia", source, t.TempDir(), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(filepath.Join(root, "source"))
	if err != nil || info.Mode().Perm() != 0644 {
		t.Fatalf("cloned profile permissions: %v: %v", info, err)
	}
	cmd := isolated(root, nil, "sh", "-c", "touch /tmp/compiler-output; stat -c %a /tmp/compiler-output")
	if out, err := cmd.CombinedOutput(); err != nil || strings.TrimSpace(string(out)) != "644" {
		t.Fatalf("compiler output permissions: %s: %v", out, err)
	}
	info, err = os.Stat(filepath.Join(source.Path, "source"))
	if err != nil || info.Mode().Perm() != 0664 {
		t.Fatal("changed original source permissions")
	}
}

func TestStageLogRetainsFailureAndElapsedTime(t *testing.T) {
	path := filepath.Join(t.TempDir(), "stage.log")
	if err := logged(buildCommand("sh", "-c", "echo compiler-diagnostic; exit 7"), path); err == nil {
		t.Fatal("lost command failure")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	for _, text := range []string{"compiler-diagnostic", "Started:", "elapsed=", "status=FAIL"} {
		if !strings.Contains(string(data), text) {
			t.Fatalf("missing %s: %s", text, data)
		}
	}
}
