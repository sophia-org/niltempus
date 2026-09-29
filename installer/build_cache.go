package main

import (
	"os"
	"path/filepath"
)

// The caller holds buildLock across compilation and packaging. Cargo still
// checks the selected sources and toolchain on every run; only its target
// directories survive retries. Logs, signed source staging and release output
// remain private to each attempt.
func packageBuildTargets(cache, work string) (string, string, error) {
	base := filepath.Join(cache, "package-targets")
	if err := ownedDirectory(cache); err != nil {
		return "", "", err
	}
	if err := ownedDirectory(base); err != nil {
		return "", "", err
	}
	build := filepath.Join(work, "package-build")
	if err := os.Mkdir(build, 0700); err != nil {
		return "", "", err
	}
	for _, name := range []string{"integration-bootstrap", "sophia-target", "integration-target"} {
		target := filepath.Join(base, name)
		if err := ownedDirectory(target); err != nil {
			return "", "", err
		}
		if name != "integration-bootstrap" {
			if err := os.Symlink(target, filepath.Join(build, name)); err != nil {
				return "", "", err
			}
		}
	}
	return filepath.Join(base, "integration-bootstrap"), build, nil
}
