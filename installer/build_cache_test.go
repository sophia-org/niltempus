package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPackageRetriesReuseTargetsAndKeepAttemptLogsSeparate(t *testing.T) {
	cache := t.TempDir()
	first, build1, err := packageBuildTargets(cache, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(build1, "sophia-build.log"), []byte("failed attempt"), 0600); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"sophia-target", "integration-target"} {
		if err := os.WriteFile(filepath.Join(build1, name, "cached"), []byte(name), 0600); err != nil {
			t.Fatal(err)
		}
	}
	second, build2, err := packageBuildTargets(cache, t.TempDir())
	if err != nil || first != second || build1 == build2 {
		t.Fatalf("retry paths: %s %s %s %s: %v", first, second, build1, build2, err)
	}
	for _, name := range []string{"sophia-target", "integration-target"} {
		data, err := os.ReadFile(filepath.Join(build2, name, "cached"))
		if err != nil || string(data) != name {
			t.Fatalf("lost cached target %s: %q %v", name, data, err)
		}
	}
	if _, err := os.Stat(filepath.Join(build2, "sophia-build.log")); !os.IsNotExist(err) {
		t.Fatal("retry inherited an old build log", err)
	}
}

func TestPackageCacheRefusesRedirectedOrSharedTargets(t *testing.T) {
	for _, kind := range []string{"symlink", "shared"} {
		t.Run(kind, func(t *testing.T) {
			cache := t.TempDir()
			base := filepath.Join(cache, "package-targets")
			if err := os.Mkdir(base, 0700); err != nil {
				t.Fatal(err)
			}
			target := filepath.Join(base, "sophia-target")
			if kind == "symlink" {
				if err := os.Symlink(t.TempDir(), target); err != nil {
					t.Fatal(err)
				}
			} else {
				if err := os.Mkdir(target, 0700); err != nil {
					t.Fatal(err)
				}
				if err := os.Chmod(target, 0777); err != nil {
					t.Fatal(err)
				}
			}
			if _, _, err := packageBuildTargets(cache, t.TempDir()); err == nil {
				t.Fatal("accepted unsafe cache", kind)
			}
		})
	}
}
