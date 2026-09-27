package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPreparedReleaseRefusesChangedSelectionAndArtifact(t *testing.T) {
	for _, damage := range []string{"", "binary", "resealed", "missing release", "relative", "schema", "identity", "malformed"} {
		t.Run(damage, func(t *testing.T) {
			root, plan := fixtureRelease(t)
			loc := Locations{State: t.TempDir()}
			if err := selectPreparedRelease(root, loc); err != nil {
				t.Fatal(err)
			}
			marker := filepath.Join(loc.State, "prepared.json")
			var selection PreparedRelease
			if err := readJSON(marker, &selection); err != nil {
				t.Fatal(err)
			}
			switch damage {
			case "binary", "resealed":
				if err := writeFile(filepath.Join(root, "target/release/hagia"), []byte("different"), 0755); err != nil {
					t.Fatal(err)
				}
				if damage == "resealed" {
					if err := sealRelease(root, plan); err != nil {
						t.Fatal(err)
					}
				}
			case "missing release":
				if err := os.Rename(root, root+"-moved"); err != nil {
					t.Fatal(err)
				}
				defer os.RemoveAll(root + "-moved")
			case "relative", "schema", "identity":
				switch damage {
				case "relative":
					selection.Directory = "relative"
				case "schema":
					selection.Schema = 99
				case "identity":
					selection.ReleaseID = "other"
				}
				if err := writeJSON(marker, selection); err != nil {
					t.Fatal(err)
				}
			case "malformed":
				if err := os.WriteFile(marker, []byte("{"), 0600); err != nil {
					t.Fatal(err)
				}
			}
			path, err := preparedRelease(loc)
			if damage == "" {
				if err != nil || path != root {
					t.Fatalf("valid prepared release: %s: %v", path, err)
				}
			} else if err == nil {
				t.Fatal("accepted changed prepared release")
			}
		})
	}
}

func TestFailedPreparationPreservesPreviousSelection(t *testing.T) {
	root, _ := fixtureRelease(t)
	loc := Locations{State: t.TempDir()}
	if err := selectPreparedRelease(root, loc); err != nil {
		t.Fatal(err)
	}
	// An installed-style legacy (plan schema 2) release verifies, but is not
	// a new candidate.
	bad, _ := legacyFixtureRelease(t)
	if _, err := verifyRelease(bad); err != nil {
		t.Fatal(err)
	}
	if err := selectPreparedRelease(bad, loc); err == nil || !strings.Contains(err.Error(), "plan schema 3") {
		t.Fatalf("legacy release selected for new install: %v", err)
	}
	if selected, err := preparedRelease(loc); err != nil || selected != root {
		t.Fatalf("lost previous selection: %s: %v", selected, err)
	}
}

func TestInstallDoesNotRebuildAMissingPreparedRelease(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	t.Setenv("XDG_CACHE_HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	loc, err := locations("")
	if err != nil {
		t.Fatal(err)
	}
	root, _ := fixtureRelease(t)
	if err := selectPreparedRelease(root, loc); err != nil {
		t.Fatal(err)
	}
	if err := os.RemoveAll(root); err != nil {
		t.Fatal(err)
	}
	// No config exists: falling through to a new build would fail there,
	// instead of reporting the selected artifact. No sudo is reached.
	if err := run([]string{"install"}); err == nil || !strings.Contains(err.Error(), "prepared release:") {
		t.Fatalf("missing selection triggered a rebuild: %v", err)
	}
}
