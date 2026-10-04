package main

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

func TestComponentRestoreIncludesFailedPublisherAndExactPrevious(t *testing.T) {
	for _, name := range oneShotOrder {
		t.Run(name, func(t *testing.T) {
			loc := Locations{State: t.TempDir()}
			testVersion(t, loc, name, "previous")
			testVersion(t, loc, name, "current")
			if err := os.Chmod(componentPath(loc, name), 0700); err != nil {
				t.Fatal(err)
			}
			saved, err := snapshotComponents(loc, []string{name})
			if err != nil {
				t.Fatal(err)
			}
			// The failing publisher has replaced the executable and Hagia metadata,
			// but has not renamed its pending selection record.
			testVersion(t, loc, name, "new")
			selection := filepath.Join(componentDir(loc, name), "selection.json")
			if err := os.Rename(selection, filepath.Join(componentDir(loc, name), "pending.json")); err != nil {
				t.Fatal(err)
			}
			for _, f := range saved.Files {
				if f.Path == selection {
					data, _ := os.ReadFile(f.Copy)
					if err := os.WriteFile(selection, data, f.Mode); err != nil {
						t.Fatal(err)
					}
				}
			}
			if err := saved.restore(name); err != nil {
				t.Fatal(err)
			}
			for _, f := range saved.Files {
				got, err := os.ReadFile(f.Path)
				if f.Copy == "" {
					if !os.IsNotExist(err) {
						t.Fatalf("left pending state: %s", f.Path)
					}
					continue
				}
				want, _ := os.ReadFile(f.Copy)
				info, statErr := os.Stat(f.Path)
				if err != nil || statErr != nil || !bytes.Equal(got, want) || info.Mode().Perm() != f.Mode {
					t.Fatalf("not restored exactly: %s", f.Path)
				}
			}
			if _, err := componentSelection(loc, name); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestComponentRestoreRemovesFailedFirstKleisSelection(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	saved, err := snapshotComponents(loc, []string{"kleis"})
	if err != nil {
		t.Fatal(err)
	}
	testVersion(t, loc, "kleis", "new")
	if err := os.WriteFile(filepath.Join(componentDir(loc, "kleis"), "pending.json"), []byte("interrupted"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := saved.restore("kleis"); err != nil {
		t.Fatal(err)
	}
	for _, path := range componentMutablePaths(loc, "kleis") {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatalf("left first selection: %s", path)
		}
	}
}

func TestComponentSnapshotRefusesPendingAndIncompleteState(t *testing.T) {
	for _, state := range []string{"pending", "current-only", "selection-only"} {
		t.Run(state, func(t *testing.T) {
			loc := Locations{State: t.TempDir()}
			testVersion(t, loc, "kleis", "old")
			switch state {
			case "pending":
				os.WriteFile(filepath.Join(componentDir(loc, "kleis"), "pending.json"), []byte("pending"), 0600)
			case "current-only":
				os.Remove(filepath.Join(componentDir(loc, "kleis"), "selection.json"))
			case "selection-only":
				os.Remove(componentPath(loc, "kleis"))
			}
			if _, err := snapshotComponents(loc, []string{"kleis"}); err == nil {
				t.Fatal("accepted incomplete state")
			}
		})
	}
}

func TestOneShotRetainsPersonalLomOnlyForUnchangedPackage(t *testing.T) {
	a := Plan{Sources: map[string]Source{"lom": {Repository: Repository{Path: "/lom"}, Commit: "old"}}}
	b := Plan{Sources: map[string]Source{"lom": {Repository: Repository{Path: "/lom", Reference: "different spelling"}, Commit: "old"}}}
	if !unchangedPackagedComponent("lom", a, b) {
		t.Fatal("would overwrite independent Lom update")
	}
	b.Sources["lom"] = Source{Repository: Repository{Path: "/lom"}, Commit: "new"}
	if unchangedPackagedComponent("lom", a, b) || unchangedPackagedComponent("lom", Plan{}, b) {
		t.Fatal("would ignore new package")
	}
}

func TestRetainedCComponentsRequireTheNewSDK(t *testing.T) {
	for _, name := range []string{"hagia", "bemenu"} {
		read := func(Source, string) ([]byte, error) { return []byte("old sdk"), nil }
		if retainedComponentCompatible(name, Source{}, []byte("new sdk"), read) {
			t.Fatal("retained incompatible personal", name)
		}
		if !retainedComponentCompatible(name, Source{}, []byte("old sdk"), read) {
			t.Fatal("discarded compatible personal", name)
		}
	}
}
