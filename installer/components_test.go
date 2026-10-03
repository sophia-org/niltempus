package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func testVersion(t *testing.T, loc Locations, name, text string) ComponentVersion {
	t.Helper()
	path := filepath.Join(t.TempDir(), "binary")
	if err := os.WriteFile(path, []byte(text), 0755); err != nil {
		t.Fatal(err)
	}
	v := ComponentVersion{name, Source{Commit: strings.Repeat("a", 40), Signature: "G"}, digest([]byte(text))}
	if err := selectComponent(loc, v, path); err != nil {
		t.Fatal(err)
	}
	return v
}
func TestComponentSelectionsAreIndependentAndRollbackRetainsVersions(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		testVersion(t, loc, name, "first-"+name)
	}
	wm, _ := os.Stat(personalHagia(loc))
	menu, _ := os.Stat(componentPath(loc, "bemenu"))
	first, _ := componentSelection(loc, "lom")
	next := testVersion(t, loc, "lom", "new bar")
	selected, err := componentSelection(loc, "lom")
	if err != nil || selected.Current != next || selected.Previous == nil || *selected.Previous != first.Current {
		t.Fatalf("selection: %+v %v", selected, err)
	}
	if err := rollbackComponent(loc, "lom"); err != nil {
		t.Fatal(err)
	}
	selected, err = componentSelection(loc, "lom")
	if err != nil || selected.Current != first.Current {
		t.Fatalf("rollback: %+v %v", selected, err)
	}
	for path, before := range map[string]os.FileInfo{personalHagia(loc): wm, componentPath(loc, "bemenu"): menu} {
		after, _ := os.Stat(path)
		if !os.SameFile(before, after) {
			t.Fatal("neighbour replaced", path)
		}
	}
	if hash, err := fileDigest(filepath.Join(componentDir(loc, "lom"), "versions", next.SHA256)); err != nil || hash != next.SHA256 {
		t.Fatal("new version lost after rollback")
	}
}
func TestComponentRefusesCorruptionAndFailedPublicationPreservesSelection(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	old := testVersion(t, loc, "lom", "old")
	candidate := filepath.Join(t.TempDir(), "bad")
	os.WriteFile(candidate, []byte("bad"), 0755)
	v := old
	v.SHA256 = digest([]byte("expected"))
	if err := selectComponent(loc, v, candidate); err == nil {
		t.Fatal("accepted wrong digest")
	}
	if s, err := componentSelection(loc, "lom"); err != nil || s.Current != old {
		t.Fatal("failed build changed selection", err)
	}
	os.WriteFile(componentPath(loc, "lom"), []byte("tamper"), 0755)
	if _, err := componentSelection(loc, "lom"); err == nil {
		t.Fatal("accepted changed executable")
	}
	if err := rollbackComponent(loc, "../../outside"); err == nil {
		t.Fatal("accepted traversal")
	}
}
func TestComponentProfileKeepsRoleTransportAndSettings(t *testing.T) {
	source := `session {
 window-manager "/sealed/hagia"
 shell-component "bar" "bar" {
  executable "/sealed/lom"
  transport "9p2000.L"
  gpu "direct"
  config "/unchanged/lom.kdl"
 }
 shell-component "menu" "application-launcher" {
  executable "/sealed/bemenu"
  transport "9p2000.L"
 }
}
`
	loc := Locations{State: "/user with spaces/state"}
	out, err := renderComponentProfile(source, loc)
	if err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{componentPath(loc, "hagia"), componentPath(loc, "lom"), componentPath(loc, "bemenu"), `gpu direct`, `config "/unchanged/lom.kdl"`} {
		if !strings.Contains(out, value) {
			t.Fatal("lost", value, out)
		}
	}
	if strings.Contains(out, "/sealed/") {
		t.Fatal("still using sealed executable")
	}
	if err := requireNinePProfile(out); err != nil {
		t.Fatal(err)
	}
}
func TestSeedComponentsPreservesSelectedBinaries(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		testVersion(t, loc, name, "user-"+name)
	}
	before, _ := os.Stat(componentPath(loc, "lom"))
	if err := seedComponents("/nonexistent", Manifest{}, loc); err != nil {
		t.Fatal(err)
	}
	after, _ := os.Stat(componentPath(loc, "lom"))
	if !os.SameFile(before, after) {
		t.Fatal("install replaced selection")
	}
}

func TestRollbackCannotSelectAnotherComponent(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	testVersion(t, loc, "lom", "old")
	testVersion(t, loc, "lom", "new")
	path := filepath.Join(componentDir(loc, "lom"), "selection.json")
	var selection ComponentSelection
	if err := readJSON(path, &selection); err != nil {
		t.Fatal(err)
	}
	selection.Previous.Name = "hagia"
	if err := writeJSON(path, selection); err != nil {
		t.Fatal(err)
	}
	if err := rollbackComponent(loc, "lom"); err == nil {
		t.Fatal("cross-component rollback accepted")
	}
	if _, err := os.Lstat(personalHagia(loc)); !os.IsNotExist(err) {
		t.Fatal("rollback created another component")
	}
}

func TestVersionStoreRefusesSymlinkAndUnrecordedExecutable(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	candidate := filepath.Join(t.TempDir(), "binary")
	os.WriteFile(candidate, []byte("candidate"), 0755)
	v := ComponentVersion{"lom", Source{Commit: strings.Repeat("b", 40), Signature: "G"}, digest([]byte("candidate"))}
	dir := filepath.Join(componentDir(loc, "lom"), "versions")
	if err := ownedDirectory(dir); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(candidate, filepath.Join(dir, v.SHA256)); err != nil {
		t.Fatal(err)
	}
	if err := selectComponent(loc, v, candidate); err == nil {
		t.Fatal("followed retained version symlink")
	}
	os.Remove(filepath.Join(dir, v.SHA256))
	os.WriteFile(componentPath(loc, "lom"), []byte("unrecorded"), 0755)
	if err := selectComponent(loc, v, candidate); err == nil {
		t.Fatal("overwrote unrecorded executable")
	}
}

func TestUnsignedComponentRefusesBeforeAnyBuildOrSelection(t *testing.T) {
	source := sourceFixture(t)
	loc := Locations{Config: filepath.Join(t.TempDir(), "config.json"), State: filepath.Join(t.TempDir(), "absent-state"), Cache: filepath.Join(t.TempDir(), "absent-cache")}
	cfg := Config{Repositories: map[string]Repository{"lom": {source.Path, source.Commit}}}
	if err := writeJSON(loc.Config, cfg); err != nil {
		t.Fatal(err)
	}
	if _, err := prepareComponent(loc, "lom"); err == nil || !strings.Contains(err.Error(), "trusted signed commit") {
		t.Fatal("unsigned input accepted", err)
	}
	for _, path := range []string{loc.State, loc.Cache} {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatal("refusal staged data", path, err)
		}
	}
}

func TestComponentProfileSelectsKleisAsTheLockProvider(t *testing.T) {
	base := `session {
 window-manager "/sealed/hagia"
 shell-component "bar" "bar" {
  executable "/sealed/lom"
 }
 shell-component "menu" "application-launcher" {
  executable "/sealed/bemenu"
 }
`
	loc := Locations{State: "/state"}
	out, err := renderComponentProfile(base+` lock-provider {
  executable "/placeholder/kleis"
  config "/home/user/.config/kleis/config.kdl"
  gpu "denied"
 }
}
`, loc)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{componentPath(loc, "kleis"), `config "/home/user/.config/kleis/config.kdl"`, `gpu denied`} {
		if !strings.Contains(out, want) {
			t.Fatal("lost", want, out)
		}
	}
	if strings.Contains(out, "/placeholder/") {
		t.Fatal("kept the placeholder executable", out)
	}
	// A profile without a lock provider gains none.
	out, err = renderComponentProfile(base+"}\n", loc)
	if err != nil || strings.Contains(out, "lock-provider") {
		t.Fatal("added a lock provider", err, out)
	}
	if _, err := renderComponentProfile(base+` lock-provider { executable "/a"; }
 lock-provider { executable "/b"; }
}
`, loc); err == nil {
		t.Fatal("accepted two lock providers")
	}
}

func TestLockProviderValidationProfile(t *testing.T) {
	out, err := renderLockProviderProfile("session {\n window-manager \"/wm\"\n}\n", "/candidate/kleis")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, `executable "/candidate/kleis"`) || !strings.Contains(out, `gpu denied`) {
		t.Fatal("did not add the candidate as the lock provider", out)
	}
	out, err = renderLockProviderProfile("session {\n lock-provider {\n  executable \"/old\"\n  gpu \"direct\"\n }\n}\n", "/candidate/kleis")
	if err != nil || !strings.Contains(out, `executable "/candidate/kleis"`) || !strings.Contains(out, `gpu direct`) || strings.Contains(out, "/old") {
		t.Fatal("did not point the existing provider at the candidate", err, out)
	}
}

func TestKleisIsACSDKComponent(t *testing.T) {
	if b, err := componentBinary("kleis"); err != nil || b != "kleis" {
		t.Fatal(b, err)
	}
	for name, want := range map[string]bool{"hagia": true, "bemenu": true, "kleis": true, "lom": false} {
		if usesCSDK(name) != want {
			t.Fatal(name)
		}
	}
}

func TestKleisIsNeverRestartedByTheUpdater(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	for _, action := range []string{"reload", "restart"} {
		err := componentCommand(action, "kleis", loc)
		if err == nil || !strings.Contains(err.Error(), "started by Sophia") {
			t.Fatal(action, err)
		}
	}
}
