package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Called only while holding buildLock. Stable paths and untouched mtimes let
// Cargo and Nim decide which inputs changed, including across source revisions.
func buildSource(name string, source Source, cache, work string) (string, error) {
	root := filepath.Join(cache, "sources", name+"-"+digest([]byte(source.Path))[:16])
	if name == "bemenu" {
		// Make does not fingerprint compiler flags (including the embedded Git
		// identity). Keep this small C build fresh instead of trusting stale .o's.
		root = filepath.Join(work, name)
	}
	info, err := os.Lstat(root)
	if os.IsNotExist(err) {
		if err := os.MkdirAll(filepath.Dir(root), 0700); err != nil {
			return "", err
		}
		if err := checked(buildCommand("git", "clone", "--quiet", "--shared", "--no-checkout", source.Path, root)); err != nil {
			return "", err
		}
	} else if err != nil {
		return "", err
	} else {
		if !info.IsDir() {
			return "", fmt.Errorf("build source is not a directory: %s", root)
		}
		origin, err := git(root, "remote", "get-url", "origin")
		if err != nil || origin != source.Path {
			return "", fmt.Errorf("build source origin differs from plan: %s", root)
		}
		status, err := git(root, "status", "--porcelain")
		if err != nil {
			return "", err
		}
		if status != "" {
			return "", fmt.Errorf("refusing changed build checkout: %s; inspect it before removing this disposable cache", root)
		}
	}
	if _, err := git(root, "checkout", "--quiet", "--detach", source.Commit); err != nil {
		return "", err
	}
	return root, nil
}

func linkBuildTarget(root, target string) error {
	path := filepath.Join(root, "target")
	if _, err := os.Lstat(path); os.IsNotExist(err) {
		if err := os.Symlink(target, path); err != nil {
			return err
		}
	} else if err != nil {
		return err
	} else if actual, err := os.Readlink(path); err != nil || actual != target {
		return fmt.Errorf("unexpected build target at %s", path)
	}
	// Git's target/ ignore rule does not match a symlink. Keep this exception
	// local to the private checkout, and keep repeated builds byte-stable.
	path = filepath.Join(root, ".git/info/exclude")
	data, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	if !strings.Contains("\n"+string(data)+"\n", "\n/target\n") {
		return writeFile(path, append(data, []byte("\n/target\n")...), 0644)
	}
	return nil
}
