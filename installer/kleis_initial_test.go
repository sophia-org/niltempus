package main

import (
	"fmt"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestPlanAllowsOptionalComponentRepository(t *testing.T) {
	source := sourceFixture(t)
	profile := filepath.Join(t.TempDir(), "desktop.kdl")
	if err := writeFile(profile, []byte(fixtureProfile), 0600); err != nil {
		t.Fatal(err)
	}
	cfg := Config{Profile: profile, Repositories: map[string]Repository{}}
	for _, name := range components {
		cfg.Repositories[name] = Repository{source.Path, source.Commit}
	}
	before, err := createSourcePlan(cfg)
	if err != nil {
		t.Fatal(err)
	}
	cfg.Repositories["kleis"] = Repository{source.Path, source.Commit}
	after, err := createSourcePlan(cfg)
	if err != nil {
		t.Fatalf("optional kleis rejected: %v", err)
	}
	if after.ReleaseID != before.ReleaseID || !maps.Equal(after.Sources, before.Sources) {
		t.Fatal("component-only source changed packaged sources or release identity")
	}
	for _, extra := range []bool{false, true} {
		bad := cfg
		bad.Repositories = maps.Clone(cfg.Repositories)
		bad.Repositories["unknown"] = bad.Repositories["kleis"]
		if !extra {
			delete(bad.Repositories, "kleis")
		}
		if _, err := createSourcePlan(bad); err == nil {
			t.Fatal("accepted an unknown repository")
		}
	}
	delete(cfg.Repositories, "sophia")
	if _, err := createSourcePlan(cfg); err == nil {
		t.Fatal("optional repository substituted for a required one")
	}
}

// Exercise the real command boundary with a verified fixture at a private
// /opt. An unsigned source stops preparation before any build or publication.
func TestFirstKleisPreparationReachesSourceValidation(t *testing.T) {
	release, _ := fixtureRelease(t)
	source := sourceFixture(t)
	fixture := t.TempDir()
	if err := writeJSON(filepath.Join(fixture, "config.json"), Config{
		Repositories: map[string]Repository{"kleis": {source.Path, source.Commit}},
	}); err != nil {
		t.Fatal(err)
	}
	binary, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	args := []string{"--die-with-parent", "--unshare-pid", "--unshare-net",
		"--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
		"--tmpfs", "/tmp", "--tmpfs", "/run/user", "--tmpfs", "/opt",
		"--ro-bind", release, filepath.Join(prefix, "current"),
		"--ro-bind", source.Path, source.Path, "--bind", fixture, "/tmp/fixture",
		"--ro-bind", binary, "/tmp/test-runner", "--chdir", "/tmp/fixture",
		"--setenv", "NILTEMPUS_TEST_INITIAL_KLEIS", "1", "--",
		"/tmp/test-runner", "-test.run=^TestKleisPreparationCommandHelper$", "-test.v"}
	cmd := exec.Command("bwrap", args...)
	cmd.Env = buildEnvironment(os.Environ())
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("initial preparation refused before source validation: %v\n%s", err, out)
	}
	for _, name := range []string{"state", "cache"} {
		if _, err := os.Lstat(filepath.Join(fixture, name)); !os.IsNotExist(err) {
			t.Fatalf("refused preparation changed %s: %v", name, err)
		}
	}
}

func TestKleisPreparationCommandHelper(t *testing.T) {
	if os.Getenv("NILTEMPUS_TEST_INITIAL_KLEIS") != "1" {
		t.Skip("private command subprocess")
	}
	loc := Locations{Config: "/tmp/fixture/config.json", State: "/tmp/fixture/state", Cache: "/tmp/fixture/cache"}
	err := componentCommand("prepare-component", "kleis", loc)
	if err == nil || !strings.Contains(err.Error(), "trusted signed commit") {
		t.Fatalf("first preparation must reach source validation: %v", err)
	}
	fmt.Println("first kleis preparation: unsigned source refused before publication")
}

func TestKleisPreparationRefusesIncompleteSelections(t *testing.T) {
	for _, damage := range []string{"missing-current", "missing-selection", "bad-hash", "bad-json", "pending", "only-pending", "unrecorded-symlink"} {
		t.Run(damage, func(t *testing.T) {
			loc := Locations{State: t.TempDir()}
			dir := componentDir(loc, "kleis")
			if damage != "only-pending" && damage != "unrecorded-symlink" {
				testVersion(t, loc, "kleis", "existing executable")
			}
			var err error
			switch damage {
			case "missing-current":
				err = os.Remove(componentPath(loc, "kleis"))
			case "missing-selection":
				err = os.Remove(filepath.Join(dir, "selection.json"))
			case "bad-hash":
				err = os.WriteFile(componentPath(loc, "kleis"), []byte("changed"), 0755)
			case "bad-json":
				err = os.WriteFile(filepath.Join(dir, "selection.json"), []byte("{"), 0600)
			case "pending", "only-pending":
				err = writeFile(filepath.Join(dir, "pending.json"), []byte("recovery record"), 0600)
			case "unrecorded-symlink":
				if err = ownedDirectory(dir); err == nil {
					err = os.Symlink("missing", componentPath(loc, "kleis"))
				}
			}
			if err != nil {
				t.Fatal(err)
			}
			if err := componentCommandSelection(loc, "kleis", true); err == nil {
				t.Fatal("incomplete selection treated as an initial preparation")
			}
		})
	}
	loc := Locations{State: t.TempDir()}
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		if err := componentCommandSelection(loc, name, true); err == nil {
			t.Fatal("relaxed existing component initialization", name)
		}
	}
	if err := componentCommandSelection(loc, "kleis", false); err == nil {
		t.Fatal("an unselected component can only be prepared")
	}
}

func TestKleisSelectionIsIdempotentAndFailedPublicationPreservesIt(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	old := testVersion(t, loc, "kleis", "first kleis")
	before, err := os.Stat(componentPath(loc, "kleis"))
	if err != nil {
		t.Fatal(err)
	}
	version := filepath.Join(componentDir(loc, "kleis"), "versions", old.SHA256)
	if err := componentCommandSelection(loc, "kleis", true); err != nil {
		t.Fatal(err)
	}
	if err := selectComponent(loc, old, version); err != nil {
		t.Fatal(err)
	}
	after, err := os.Stat(componentPath(loc, "kleis"))
	if err != nil || !os.SameFile(before, after) {
		t.Fatal("repeated selection replaced the executable", err)
	}
	bad := old
	bad.SHA256 = strings.Repeat("b", 64)
	if err := selectComponent(loc, bad, version); err == nil {
		t.Fatal("accepted a candidate with the wrong digest")
	}
	selected, err := componentSelection(loc, "kleis")
	if err != nil || selected.Current != old || selected.Previous != nil {
		t.Fatalf("failed publication changed the initial selection: %+v %v", selected, err)
	}
}
