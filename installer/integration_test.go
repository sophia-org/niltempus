package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestIntegrationProvisionRequiresTheSelectedPinAndLock(t *testing.T) {
	rev := strings.Repeat("a", 40)
	lock := []byte("version = 4\n")
	pin := []byte("# Source pin\nurl = \"" + sophiaRepositoryURL + "\"\nrev = \"" + rev + "\"\n")
	marker := "url=" + sophiaRepositoryURL + "\nrev=" + rev + "\ncargo_lock_sha256=" + digest(lock) + "\ncargo_home=/private/cache\n"
	home, err := validateIntegrationProvision(pin, lock, []byte(marker), rev)
	if err != nil || home != "/private/cache" {
		t.Fatalf("valid provisioning: %s: %v", home, err)
	}
	for name, mutate := range map[string]func() ([]byte, []byte, []byte, string){
		"selected Sophia differs": func() ([]byte, []byte, []byte, string) { return pin, lock, []byte(marker), strings.Repeat("b", 40) },
		"stale lock": func() ([]byte, []byte, []byte, string) {
			return pin, append(append([]byte{}, lock...), '\n'), []byte(marker), rev
		},
		"stale marker rev": func() ([]byte, []byte, []byte, string) {
			return pin, lock, []byte(strings.Replace(marker, rev, strings.Repeat("b", 40), 1)), rev
		},
		"local URL": func() ([]byte, []byte, []byte, string) {
			return []byte(strings.ReplaceAll(string(pin), sophiaRepositoryURL, "file:///source")), lock, []byte(strings.ReplaceAll(marker, sophiaRepositoryURL, "file:///source")), rev
		},
		"relative cache": func() ([]byte, []byte, []byte, string) {
			return pin, lock, []byte(strings.Replace(marker, "/private/cache", "cache", 1)), rev
		},
		"missing digest": func() ([]byte, []byte, []byte, string) {
			return pin, lock, []byte(strings.Replace(marker, "cargo_lock_sha256="+digest(lock)+"\n", "", 1)), rev
		},
		"duplicate":   func() ([]byte, []byte, []byte, string) { return pin, lock, []byte(marker + "rev=" + rev + "\n"), rev },
		"extra field": func() ([]byte, []byte, []byte, string) { return pin, lock, []byte(marker + "extra=accepted\n"), rev },
	} {
		t.Run(name, func(t *testing.T) {
			p, l, m, selected := mutate()
			if _, err := validateIntegrationProvision(p, l, m, selected); err == nil {
				t.Fatal("accepted mismatched provisioning")
			}
		})
	}
}

func TestIntegrationHomeCannotBeInASourceTreeOrBeASymlink(t *testing.T) {
	source := t.TempDir()
	home := t.TempDir()
	if err := integrationHome(home, source); err != nil {
		t.Fatal(err)
	}
	nested := filepath.Join(source, "cargo-home")
	if err := os.Mkdir(nested, 0700); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(t.TempDir(), "linked-cache")
	if err := os.Symlink(home, link); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{source, nested, filepath.Dir(source), link, filepath.Join(source, "missing")} {
		if err := integrationHome(bad, source); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	if err := os.Chmod(home, 0777); err != nil {
		t.Fatal(err)
	}
	if err := integrationHome(home, source); err == nil {
		t.Fatal("accepted a cache writable by other users")
	}
}

func TestIntegrationRejectsUnsignedPackagerBeforeReadingProvisioning(t *testing.T) {
	source := sourceFixture(t)
	_, err := planIntegration(Repository{source.Path, source.Commit}, source)
	if err == nil || !strings.Contains(err.Error(), "trusted signed commit") {
		t.Fatalf("unsigned source: %v", err)
	}
}

func TestHistoricalPlanIdentityExcludesAbsentIntegration(t *testing.T) {
	const oldJSON = `{"schema":2,"release_id":"","sources":{},"profile":"/profile","profile_sha256":"profile","installer_sha256":"installer"}`
	var plan Plan
	if err := json.Unmarshal([]byte(oldJSON), &plan); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(plan)
	if err != nil || string(encoded) != oldJSON {
		t.Fatalf("historical identity changed: %s: %v", encoded, err)
	}
	oldID := releaseID(plan)
	plan.Integration = &IntegrationPlan{Source: Source{Commit: "integration"}, CargoHome: "/cache", CargoLockSHA256: "lock"}
	if releaseID(plan) == oldID {
		t.Fatal("external packager is absent from new release identity")
	}
	newID := releaseID(plan)
	for _, change := range []func(*IntegrationPlan){
		func(p *IntegrationPlan) { p.Source.Commit = "other" },
		func(p *IntegrationPlan) { p.CargoHome = "/another-cache" },
		func(p *IntegrationPlan) { p.CargoLockSHA256 = "different-lock" },
	} {
		copy := *plan.Integration
		change(&copy)
		candidate := plan
		candidate.Integration = &copy
		if releaseID(candidate) == newID {
			t.Fatal("integration input does not affect release identity")
		}
	}
}

func TestUnbuiltHistoricalPlanCannotBypassExternalPackager(t *testing.T) {
	_, err := buildRelease(Plan{ReleaseID: "old-plan"}, Locations{State: t.TempDir()})
	if err == nil || !strings.Contains(err.Error(), "external integration packager") {
		t.Fatalf("unbound new build: %v", err)
	}
}

func TestPersonalWMPreparationDoesNotRequireIntegrationProvisioning(t *testing.T) {
	source := sourceFixture(t)
	config := Config{Repositories: map[string]Repository{"hagia": {source.Path, source.Commit}}}
	loc := Locations{Config: filepath.Join(t.TempDir(), "config.json")}
	if err := writeJSON(loc.Config, config); err != nil {
		t.Fatal(err)
	}
	selected, err := developmentSource(loc)
	if err != nil || selected.Commit != source.Commit || selected.Path != source.Path {
		t.Fatalf("WM-only source selection needs desktop provisioning: %+v: %v", selected, err)
	}
}

func TestExternalReleaseRequiresItsToolsAndMatchingProvenance(t *testing.T) {
	for _, damage := range []string{"", "missing tool", "wrong integration", "missing source", "wrong pair digest"} {
		t.Run(damage, func(t *testing.T) {
			root, plan := fixtureRelease(t)
			plan.Integration = &IntegrationPlan{Source: Source{Commit: "packager"}, CargoHome: "/cache", CargoLockSHA256: "lock"}
			plan.Sources = map[string]Source{"sophia": {Commit: "sophia"}, "hagia": {Commit: "hagia"}, "narthex": {Commit: "narthex"}}
			plan.ReleaseID = releaseID(plan)
			for _, path := range []string{"target/release/sophia-integration-xtask", "target/release/active-session-preflight", "tools/session/run_desktop_session.sh", "bin/sophia-session"} {
				if err := writeFile(filepath.Join(root, path), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
					t.Fatal(err)
				}
			}
			if err := writeFile(filepath.Join(root, "share/sophia-policy/hagia/default.kdl"), []byte("profile"), 0644); err != nil {
				t.Fatal(err)
			}
			binaryHash, err := fileDigest(filepath.Join(root, "target/release/hagia"))
			if err != nil {
				t.Fatal(err)
			}
			metadata := "schema=6\nrelease_id=" + plan.ReleaseID + "\ncommit=sophia\nintegration_commit=packager\nhagia_source_commit=hagia\nnarthex_source_commit=narthex\nhagia_binary_sha256=" + binaryHash + "\nhagia_shell_binary_sha256=" + binaryHash + "\nhagia_default_profile_sha256=" + digest([]byte("profile")) + "\n"
			switch damage {
			case "missing tool":
				if err := os.Remove(filepath.Join(root, "target/release/active-session-preflight")); err != nil {
					t.Fatal(err)
				}
			case "wrong integration":
				metadata = strings.Replace(metadata, "integration_commit=packager", "integration_commit=other", 1)
			case "missing source":
				metadata = strings.Replace(metadata, "hagia_source_commit=hagia\n", "", 1)
			case "wrong pair digest":
				metadata = strings.Replace(metadata, "hagia_binary_sha256="+binaryHash, "hagia_binary_sha256=wrong", 1)
			}
			if err := writeFile(filepath.Join(root, "manifest"), []byte(metadata), 0644); err != nil {
				t.Fatal(err)
			}
			if err := sealRelease(root, plan); err != nil {
				t.Fatal(err)
			}
			_, err = verifyRelease(root)
			if (err != nil) != (damage != "") {
				t.Fatalf("verify %s: %v", damage, err)
			}
		})
	}
}
