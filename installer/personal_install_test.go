package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func personalPackage(t *testing.T, text string) (string, Manifest) {
	t.Helper()
	artifact := t.TempDir()
	if err := writeFile(filepath.Join(artifact, "target/release/hagia"), []byte(text), 0755); err != nil {
		t.Fatal(err)
	}
	return artifact, Manifest{
		Plan:  Plan{Sources: map[string]Source{"hagia": {Commit: text}}},
		Files: map[string]FileRecord{"target/release/hagia": {SHA256: digest([]byte(text)), Mode: 0755}},
	}
}

func TestDesktopInstallationPreservesPersonalWMAndMetadata(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	binary := personalHagia(loc)
	metadata := filepath.Join(filepath.Dir(binary), "hagia.json")
	for _, path := range []string{binary, metadata} {
		mode := os.FileMode(0600)
		if path == binary {
			mode = 0700
		}
		if err := writeFile(path, []byte("user-selected "+filepath.Base(path)), mode); err != nil {
			t.Fatal(err)
		}
		stamp := time.Unix(1000000000, 0)
		if err := os.Chtimes(path, stamp, stamp); err != nil {
			t.Fatal(err)
		}
	}
	before := make(map[string]os.FileInfo)
	contents := make(map[string]string)
	for _, path := range []string{binary, metadata} {
		var err error
		before[path], err = os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		contents[path] = string(data)
	}
	// The same helper is used for install, repeat install and rollback.
	for _, revision := range []string{"new-release", "new-release", "previous-release"} {
		artifact, manifest := personalPackage(t, revision)
		if err := installPersonalHagia(artifact, manifest, loc); err != nil {
			t.Fatal(err)
		}
		for _, path := range []string{binary, metadata} {
			after, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			data, err := os.ReadFile(path)
			if err != nil || string(data) != contents[path] || !os.SameFile(before[path], after) || before[path].Mode() != after.Mode() || !before[path].ModTime().Equal(after.ModTime()) {
				t.Fatalf("%s changed personal state %s: %v", revision, path, err)
			}
		}
	}
}

func TestDesktopInstallationSeedsOnlyAnAbsentPersonalWM(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	artifact, manifest := personalPackage(t, "initial")
	bad := manifest
	bad.Files = map[string]FileRecord{"target/release/hagia": {SHA256: digest([]byte("changed"))}}
	if err := installPersonalHagia(artifact, bad, loc); err == nil {
		t.Fatal("initialized from a changed package")
	}
	if _, err := os.Lstat(personalHagia(loc)); !os.IsNotExist(err) {
		t.Fatalf("failed initialization published a WM: %v", err)
	}
	if err := installPersonalHagia(artifact, manifest, loc); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(personalHagia(loc))
	if err != nil || string(data) != "initial" {
		t.Fatalf("initial publication: %q: %v", data, err)
	}
	var metadata DevelopmentHagia
	if err := readJSON(filepath.Join(filepath.Dir(personalHagia(loc)), "hagia.json"), &metadata); err != nil || metadata.SHA256 != digest(data) {
		t.Fatalf("initial metadata: %+v: %v", metadata, err)
	}
	entries, err := os.ReadDir(filepath.Dir(personalHagia(loc)))
	if err != nil || len(entries) != 2 {
		t.Fatalf("staging file left behind: %v: %v", entries, err)
	}
}

func TestDesktopInstallationRefusesInvalidPersonalWMWithoutReplacingIt(t *testing.T) {
	for _, mode := range []os.FileMode{0600, 0775, 0777} {
		loc := Locations{State: t.TempDir()}
		if err := writeFile(personalHagia(loc), []byte("untouched"), mode); err != nil {
			t.Fatal(err)
		}
		artifact, manifest := personalPackage(t, "package")
		if err := installPersonalHagia(artifact, manifest, loc); err == nil {
			t.Fatalf("accepted existing mode %o", mode)
		}
		if data, err := os.ReadFile(personalHagia(loc)); err != nil || string(data) != "untouched" {
			t.Fatal("refusal replaced the personal WM")
		}
	}
}
