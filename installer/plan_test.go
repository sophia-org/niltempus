package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPlanUsesSelectedCommitDespiteCheckoutAndDirtyFiles(t *testing.T) {
	root := t.TempDir()
	command := func(args ...string) string {
		t.Helper()
		out, err := git(root, args...)
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	command("init", "--quiet", "--initial-branch=master")
	command("config", "user.name", "Installer test fixture")
	command("config", "user.email", "installer@example.invalid")
	command("config", "commit.gpgsign", "false")
	path := filepath.Join(root, "tracked")
	if err := os.WriteFile(path, []byte("master"), 0644); err != nil {
		t.Fatal(err)
	}
	command("add", "tracked")
	command("commit", "--quiet", "-m", "unsigned local test fixture")
	commit := command("rev-parse", "HEAD")
	command("checkout", "--quiet", "-b", "work-in-progress")
	if err := os.WriteFile(path, []byte("another branch"), 0644); err != nil {
		t.Fatal(err)
	}
	command("commit", "--quiet", "-am", "fixture second branch")
	if err := os.WriteFile(path, []byte("uncommitted"), 0644); err != nil {
		t.Fatal(err)
	}
	profile := filepath.Join(t.TempDir(), "desktop.kdl")
	if err := os.WriteFile(profile, []byte(fixtureProfile), 0600); err != nil {
		t.Fatal(err)
	}
	config := Config{Profile: profile, Repositories: map[string]Repository{}}
	for _, name := range components {
		config.Repositories[name] = Repository{root, "master"}
	}
	loc := Locations{Config: filepath.Join(t.TempDir(), "config.json")}
	if err := writeJSON(loc.Config, config); err != nil {
		t.Fatal(err)
	}
	// Source selection is independent of the external packager's signature
	// and provisioning checks, which have their own refusal tests.
	plan, err := createSourcePlan(config)
	if err != nil {
		t.Fatal(err)
	}
	for _, source := range plan.Sources {
		if source.Commit != commit || source.Signature != "N" {
			t.Fatal(source)
		}
	}
	if plan.ProfileSHA256 != digest([]byte(fixtureProfile)) {
		t.Fatal("wrong profile hash")
	}
	if command("branch", "--show-current") != "work-in-progress" {
		t.Fatal("changed source branch")
	}
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "uncommitted" {
		t.Fatal("changed source WIP")
	}
	again, err := createSourcePlan(config)
	if err != nil || again.ReleaseID != plan.ReleaseID {
		t.Fatalf("unstable identity: %v", err)
	}
}

func TestReleaseIdentityChangesWithEverySelectedInput(t *testing.T) {
	base := Plan{Schema: 1, Sources: map[string]Source{"hagia": {Commit: "original"}}, ProfileSHA256: "profile", InstallerSHA256: "installer"}
	before := releaseID(base)
	for name, change := range map[string]func(*Plan){
		"profile":   func(p *Plan) { p.ProfileSHA256 = "changed" },
		"installer": func(p *Plan) { p.InstallerSHA256 = "changed" },
		"source":    func(p *Plan) { p.Sources = map[string]Source{"hagia": {Commit: "changed"}} },
	} {
		t.Run(name, func(t *testing.T) {
			p := base
			change(&p)
			if releaseID(p) == before {
				t.Fatal("identity unchanged")
			}
		})
	}
}
