package main

import (
	"encoding/json"
	"runtime/debug"
	"strings"
	"testing"
)

func TestNewPlansRequireOneNiltempusSource(t *testing.T) {
	repo := Repository{Path: "/sources/niltempus", Reference: "signed-revision"}
	got, err := niltempusRepository(Config{Niltempus: repo})
	if err != nil || got != repo {
		t.Fatalf("single source: %+v: %v", got, err)
	}
	for name, config := range map[string]Config{
		"missing":          {},
		"missing path":     {Niltempus: Repository{Reference: repo.Reference}},
		"missing revision": {Niltempus: Repository{Path: repo.Path}},
		"old source":       {Integration: repo},
		"both sources":     {Integration: repo, Niltempus: repo},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := niltempusRepository(config); err == nil {
				t.Fatal("accepted ambiguous or incomplete tooling source")
			}
		})
	}
}

func TestInstallerMustBeBuiltFromSelectedCleanRevision(t *testing.T) {
	rev := strings.Repeat("a", 40)
	settings := []debug.BuildSetting{{Key: "vcs", Value: "git"}, {Key: "vcs.revision", Value: rev}, {Key: "vcs.modified", Value: "false"}}
	if err := installerRevision(&debug.BuildInfo{Settings: settings}, rev); err != nil {
		t.Fatal(err)
	}
	for name, values := range map[string][]debug.BuildSetting{
		"missing":             nil,
		"unknown cleanliness": settings[:2],
		"dirty":               {settings[0], settings[1], {Key: "vcs.modified", Value: "true"}},
		"different revision":  {settings[0], {Key: "vcs.revision", Value: strings.Repeat("b", 40)}, settings[2]},
		"different VCS":       {{Key: "vcs", Value: "hg"}, settings[1], settings[2]},
		"duplicate":           append(append([]debug.BuildSetting{}, settings...), settings[1]),
	} {
		t.Run(name, func(t *testing.T) {
			if err := installerRevision(&debug.BuildInfo{Settings: values}, rev); err == nil {
				t.Fatal("accepted unbound installer")
			}
		})
	}
}

func TestHistoricalToolingBindingRetainsItsReleaseIdentity(t *testing.T) {
	const encoded = `{"schema":2,"release_id":"","sources":{},"profile":"/profile","profile_sha256":"profile","installer_sha256":"installer","integration":{"source":{"path":"/source","reference":"master","commit":"historical","signature":"G"},"cargo_home":"/cache","cargo_lock_sha256":"lock"}}`
	var plan Plan
	if err := json.Unmarshal([]byte(encoded), &plan); err != nil {
		t.Fatal(err)
	}
	got, err := json.Marshal(plan)
	if err != nil || string(got) != encoded {
		t.Fatalf("historical plan changed: %s: %v", got, err)
	}
	oldID := releaseID(plan)
	old, err := toolingBinding(plan)
	if err != nil || old != plan.Integration {
		t.Fatalf("historical binding: %+v: %v", old, err)
	}
	plan.Niltempus, plan.Integration = plan.Integration, nil
	bound, err := toolingBinding(plan)
	if err != nil || bound != plan.Niltempus || releaseID(plan) == oldID {
		t.Fatalf("new source binding not reflected in identity: %+v: %v", bound, err)
	}
	plan.Integration = plan.Niltempus
	if _, err := toolingBinding(plan); err == nil {
		t.Fatal("accepted two bindings even though their values match")
	}
}

func TestHistoricalToolingCannotBuildANewRelease(t *testing.T) {
	plan := Plan{ReleaseID: "historical", Integration: &IntegrationPlan{Source: Source{Commit: "old"}}}
	if _, err := buildRelease(plan, Locations{State: t.TempDir()}); err == nil || !strings.Contains(err.Error(), "single niltempus source") {
		t.Fatalf("historical plan admitted to new build: %v", err)
	}
}
