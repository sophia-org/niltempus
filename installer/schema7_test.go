package main

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func TestSchema7ReleaseMustBindHagiasVendoredSDK(t *testing.T) {
	other := strings.Repeat("9", 40)
	cases := map[string]func(string) string{
		"missing revision": func(m string) string { return dropLine(m, "hagia_c_sdk_revision=") },
		"missing digest":   func(m string) string { return dropLine(m, "hagia_c_sdk_manifest_sha256=") },
		"repeated revision": func(m string) string {
			return m + "hagia_c_sdk_revision=" + fixtureSDKRevision + "\n"
		},
		"malformed revision": func(m string) string {
			return strings.Replace(m, "hagia_c_sdk_revision="+fixtureSDKRevision, "hagia_c_sdk_revision=841563d", 1)
		},
		"uppercase revision": func(m string) string {
			return strings.Replace(m, "hagia_c_sdk_revision="+fixtureSDKRevision, "hagia_c_sdk_revision="+strings.ToUpper(fixtureSDKRevision), 1)
		},
		"malformed digest": func(m string) string {
			return replaceValue(m, "hagia_c_sdk_manifest_sha256", "HEAD")
		},
		"revision not the plan's": func(m string) string {
			return strings.Replace(m, "hagia_c_sdk_revision="+fixtureSDKRevision, "hagia_c_sdk_revision="+other, 1)
		},
		"digest not the sealed file's": func(m string) string {
			return replaceValue(m, "hagia_c_sdk_manifest_sha256", strings.Repeat("0", 64))
		},
		"schema 6":             func(m string) string { return strings.Replace(m, "schema=7\n", "schema=6\n", 1) },
		"missing included":     func(m string) string { return dropLine(m, "hagia_included=") },
		"repeated schema":      func(m string) string { return m + "schema=7\n" },
		"included false":       func(m string) string { return strings.Replace(m, "hagia_included=true", "hagia_included=false", 1) },
		"invalid included":     func(m string) string { return strings.Replace(m, "hagia_included=true", "hagia_included=yes", 1) },
		"wrong integration":    func(m string) string { return replaceValue(m, "integration_commit", strings.Repeat("5", 40)) },
		"missing pair binding": func(m string) string { return dropLine(m, "hagia_binary_sha256=") },
	}
	for name, mutate := range cases {
		t.Run(name, func(t *testing.T) {
			root, plan := fixtureRelease(t)
			writeSchema7Metadata(t, root, plan, mutate)
			if _, err := verifyRelease(root); err == nil {
				t.Fatal("accepted")
			}
		})
	}
	// The sealed SDK manifest: changed, naming another revision (with a
	// matching recorded digest), or missing.
	for _, damage := range []string{"changed", "other revision", "missing"} {
		t.Run("sealed "+damage, func(t *testing.T) {
			root, plan := fixtureRelease(t)
			path := filepath.Join(root, sealedCSDKManifest)
			switch damage {
			case "changed":
				if err := os.WriteFile(path, []byte(fixtureSDKManifest(fixtureSDKRevision)+" "), 0644); err != nil {
					t.Fatal(err)
				}
				if err := sealRelease(root, plan); err != nil {
					t.Fatal(err)
				}
			case "other revision":
				if err := os.WriteFile(path, []byte(fixtureSDKManifest(other)), 0644); err != nil {
					t.Fatal(err)
				}
				writeSchema7Metadata(t, root, plan, nil)
			case "missing":
				if err := os.Remove(path); err != nil {
					t.Fatal(err)
				}
				if err := sealRelease(root, plan); err != nil {
					t.Fatal(err)
				}
			}
			if _, err := verifyRelease(root); err == nil {
				t.Fatal("accepted")
			}
		})
	}
}

func TestHagiaIncludedFalseRequiresTheSDKBindingAbsent(t *testing.T) {
	root := t.TempDir()
	base := "schema=7\nhagia_included=false\n"
	if err := verifyExternalSchema7([]byte(base), root, map[string]FileRecord{}, fixtureSDKRevision); err != nil {
		t.Fatal(err)
	}
	for name, test := range map[string]struct {
		metadata string
		files    map[string]FileRecord
	}{
		"revision recorded": {base + "hagia_c_sdk_revision=" + fixtureSDKRevision + "\n", map[string]FileRecord{}},
		"digest recorded":   {base + "hagia_c_sdk_manifest_sha256=" + strings.Repeat("a", 64) + "\n", map[string]FileRecord{}},
		"sealed file":       {base, map[string]FileRecord{sealedCSDKManifest: {SHA256: strings.Repeat("a", 64)}}},
	} {
		if err := verifyExternalSchema7([]byte(test.metadata), root, test.files, fixtureSDKRevision); err == nil {
			t.Fatalf("accepted %s", name)
		}
	}
}

func reviewedManifest(t *testing.T, dir, product, commit, status string) NimDepsInput {
	t.Helper()
	path := filepath.Join(dir, product+"-"+status+".nim-deps")
	data := "nim-deps schema=1 status=" + status + "\nfor product=" + product + " source_commit=" + commit + " source_tree=" + strings.Repeat("e", 40) + " store=/store\nnote text=\"fixture\"\n"
	if err := writeFile(path, []byte(data), 0644); err != nil {
		t.Fatal(err)
	}
	return NimDepsInput{path, digest([]byte(data))}
}

func fixtureInputs(t *testing.T) (*PlanInputs, map[string]Source) {
	t.Helper()
	dir := t.TempDir()
	sources := map[string]Source{"hagia": {Commit: strings.Repeat("2", 40)}, "narthex": {Commit: strings.Repeat("3", 40)}}
	return &PlanInputs{
		HagiaNimDeps:      reviewedManifest(t, dir, "hagia", sources["hagia"].Commit, "reviewed"),
		NarthexNimDeps:    reviewedManifest(t, dir, "narthex", sources["narthex"].Commit, "reviewed"),
		HagiaCSDKRevision: fixtureSDKRevision,
	}, sources
}

func TestHelperInputsAreExplicitAndReviewed(t *testing.T) {
	inputs, sources := fixtureInputs(t)
	if err := validateInputs(inputs, sources); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	link := filepath.Join(dir, "linked.nim-deps")
	if err := os.Symlink(inputs.HagiaNimDeps.Path, link); err != nil {
		t.Fatal(err)
	}
	for name, mutate := range map[string]func(*PlanInputs){
		"no SDK revision":        func(p *PlanInputs) { p.HagiaCSDKRevision = "" },
		"short SDK revision":     func(p *PlanInputs) { p.HagiaCSDKRevision = "841563d" },
		"uppercase SDK revision": func(p *PlanInputs) { p.HagiaCSDKRevision = strings.ToUpper(fixtureSDKRevision) },
		"no hagia manifest":      func(p *PlanInputs) { p.HagiaNimDeps = NimDepsInput{} },
		"no narthex digest":      func(p *PlanInputs) { p.NarthexNimDeps.SHA256 = "" },
		"relative path":          func(p *PlanInputs) { p.HagiaNimDeps.Path = "hagia.nim-deps" },
		"unclean path": func(p *PlanInputs) {
			p.HagiaNimDeps.Path = filepath.Dir(p.HagiaNimDeps.Path) + "/./" + filepath.Base(p.HagiaNimDeps.Path)
		},
		"missing file":      func(p *PlanInputs) { p.HagiaNimDeps.Path = filepath.Join(dir, "absent") },
		"symlinked file":    func(p *PlanInputs) { p.HagiaNimDeps.Path = link },
		"digest mismatch":   func(p *PlanInputs) { p.NarthexNimDeps.SHA256 = strings.Repeat("0", 64) },
		"malformed digest":  func(p *PlanInputs) { p.NarthexNimDeps.SHA256 = "abc" },
		"manifests swapped": func(p *PlanInputs) { p.HagiaNimDeps, p.NarthexNimDeps = p.NarthexNimDeps, p.HagiaNimDeps },
		"draft manifest": func(p *PlanInputs) {
			p.HagiaNimDeps = reviewedManifest(t, dir, "hagia", sources["hagia"].Commit, "draft")
		},
		"reviewed for another commit": func(p *PlanInputs) {
			p.HagiaNimDeps = reviewedManifest(t, dir, "hagia", strings.Repeat("7", 40), "reviewed")
		},
	} {
		t.Run(name, func(t *testing.T) {
			copy := *inputs
			mutate(&copy)
			if err := validateInputs(&copy, sources); err == nil {
				t.Fatal("accepted")
			}
		})
	}
	if err := validateInputs(nil, sources); err == nil || !strings.Contains(err.Error(), "no defaults") {
		t.Fatalf("absent inputs: %v", err)
	}
}

func TestBuildRefusesMissingInputsBeforeStaging(t *testing.T) {
	inputs, sources := fixtureInputs(t)
	binding := &IntegrationPlan{Source: Source{Commit: strings.Repeat("4", 40)}}
	for name, plan := range map[string]Plan{
		"plan schema 2": {Schema: 2, ReleaseID: "legacy", Sources: sources, Niltempus: binding, Inputs: inputs},
		"no inputs":     {Schema: currentPlanSchema, ReleaseID: "no-inputs", Sources: sources, Niltempus: binding},
		"bad SDK revision": {Schema: currentPlanSchema, ReleaseID: "bad-rev", Sources: sources, Niltempus: binding,
			Inputs: &PlanInputs{inputs.HagiaNimDeps, inputs.NarthexNimDeps, "HEAD"}},
	} {
		t.Run(name, func(t *testing.T) {
			loc := Locations{State: t.TempDir(), Cache: t.TempDir()}
			if _, err := buildRelease(plan, loc); err == nil {
				t.Fatal("accepted")
			}
			for _, dir := range []string{loc.State, loc.Cache} {
				if entries, err := os.ReadDir(dir); err != nil || len(entries) != 0 {
					t.Fatalf("staged before refusing: %v %v", entries, err)
				}
			}
		})
	}
}

func TestPlanRefusesAbsentInputsBeforeTooling(t *testing.T) {
	config := Config{}
	if err := validateInputs(config.Inputs, nil); err == nil {
		t.Fatal("accepted a configuration without inputs")
	}
}

func TestHelperCommandLinesUseTheCurrentCLI(t *testing.T) {
	inputs, sources := fixtureInputs(t)
	sources["sophia"] = Source{Commit: strings.Repeat("1", 40)}
	plan := Plan{Schema: currentPlanSchema, Sources: sources, Inputs: inputs}
	roots := map[string]string{"sophia": "/src/sophia", "hagia": "/src/hagia", "narthex": "/src/narthex"}
	pair := prepareWMPairArgs("/tool/xtask", plan, roots, "/work/wm-pair", "/work/pair-build", "/work/inputs/h", "/work/inputs/n")
	want := []string{"/tool/xtask", "prepare-wm-pair", "--hagia", "/src/hagia", sources["hagia"].Commit, "--narthex", "/src/narthex", sources["narthex"].Commit, "/work/wm-pair",
		"--build-dir=/work/pair-build",
		"--hagia-nim-deps=/work/inputs/h", "--hagia-nim-deps-sha256=" + inputs.HagiaNimDeps.SHA256,
		"--narthex-nim-deps=/work/inputs/n", "--narthex-nim-deps-sha256=" + inputs.NarthexNimDeps.SHA256,
		"--hagia-c-sdk-rev=" + fixtureSDKRevision}
	if !slices.Equal(pair, want) {
		t.Fatalf("prepare-wm-pair:\n%q\n%q", pair, want)
	}
	pkg := packageDesktopArgs("/tool/xtask", plan, roots, "/work/wm-pair", [3]string{"h", "n", "p"}, "/work/package-build", "/work/package")
	want = []string{"/tool/xtask", "package-desktop", "--sophia-root=/src/sophia", "--sophia-rev=" + sources["sophia"].Commit, "--wm-pair=/work/wm-pair",
		"--wm-pair-commits=" + sources["hagia"].Commit + "," + sources["narthex"].Commit, "--wm-pair-sha256=h,n", "--wm-pair-profile-sha256=p",
		"--wm-pair-c-sdk-rev=" + fixtureSDKRevision, "--build-dir=/work/package-build", "--out=/work/package"}
	if !slices.Equal(pkg, want) {
		t.Fatalf("package-desktop:\n%q\n%q", pkg, want)
	}
	// The reviewed manifests are staged privately and re-checked.
	work := t.TempDir()
	h, n, err := stageInputs(inputs, work)
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{h, n} {
		if !strings.HasPrefix(path, filepath.Join(work, "inputs")+"/") {
			t.Fatalf("not staged privately: %s", path)
		}
	}
	changed := *inputs
	changed.HagiaNimDeps.SHA256 = strings.Repeat("0", 64)
	if _, _, err := stageInputs(&changed, t.TempDir()); err == nil {
		t.Fatal("staged a manifest with another digest")
	}
}

// installedPrefix lays out a private install prefix with the given releases
// under releases/ and the current and previous links.
func installedPrefix(t *testing.T, current, previous string, releases map[string]string) string {
	t.Helper()
	dir := t.TempDir()
	for id, source := range releases {
		target := filepath.Join(dir, "releases", id)
		if err := os.MkdirAll(filepath.Dir(target), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.Symlink(source, target); err != nil {
			t.Fatal(err)
		}
	}
	for link, id := range map[string]string{"current": current, "previous": previous} {
		if id != "" {
			if err := os.Symlink(filepath.Join("releases", id), filepath.Join(dir, link)); err != nil {
				t.Fatal(err)
			}
		}
	}
	return dir
}

func ledgerLine(t *testing.T, id, root string) string {
	t.Helper()
	digests, err := releaseDigests(root)
	if err != nil {
		t.Fatal(err)
	}
	return id + " " + digests + "\n"
}

func TestActivationHistoryMirrorsTheLedger(t *testing.T) {
	legacy, legacyPlan := legacyFixtureRelease(t)
	current, currentPlan := fixtureRelease(t)
	legacyManifest, err := verifyRelease(legacy)
	if err != nil {
		t.Fatal(err)
	}
	currentManifest, err := verifyRelease(current)
	if err != nil {
		t.Fatal(err)
	}
	releases := map[string]string{legacyPlan.ReleaseID: legacy, currentPlan.ReleaseID: current}
	legacyTarget := func(dir string) string { return filepath.Join(dir, "releases", legacyPlan.ReleaseID) }

	// A pre-ledger installation: the installed legacy release linked as
	// previous remains a rollback target under its own verification.
	dir := installedPrefix(t, currentPlan.ReleaseID, legacyPlan.ReleaseID, releases)
	if err := activationAllowed(dir, legacyTarget(dir), legacyManifest); err != nil {
		t.Fatalf("pre-ledger legacy rollback: %v", err)
	}
	// The same legacy release, installed but never linked: not a candidate.
	dir = installedPrefix(t, currentPlan.ReleaseID, "", releases)
	if err := activationAllowed(dir, legacyTarget(dir), legacyManifest); err == nil {
		t.Fatal("activated a never-activated legacy release")
	}
	// A new schema-3 candidate needs no history; this installer verified it.
	if err := activationAllowed(dir, filepath.Join(dir, "releases", currentPlan.ReleaseID), currentManifest); err != nil {
		t.Fatal(err)
	}

	// With a ledger, only entries count: links grant nothing.
	dir = installedPrefix(t, currentPlan.ReleaseID, legacyPlan.ReleaseID, releases)
	ledger := filepath.Join(dir, activationLedger)
	if err := writeFile(ledger, []byte(ledgerLine(t, currentPlan.ReleaseID, current)), 0644); err != nil {
		t.Fatal(err)
	}
	if err := activationAllowed(dir, legacyTarget(dir), legacyManifest); err == nil {
		t.Fatal("an unrecorded previous link granted activation")
	}
	if err := writeFile(ledger, []byte(ledgerLine(t, currentPlan.ReleaseID, current)+ledgerLine(t, legacyPlan.ReleaseID, legacy)), 0644); err != nil {
		t.Fatal(err)
	}
	if err := activationAllowed(dir, legacyTarget(dir), legacyManifest); err != nil {
		t.Fatalf("recorded legacy rollback: %v", err)
	}
	// The same ID with changed contents is refused, even for a candidate.
	if err := writeFile(ledger, []byte(currentPlan.ReleaseID+" "+strings.Repeat("0", 64)+" "+strings.Repeat("1", 64)+"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := activationAllowed(dir, filepath.Join(dir, "releases", currentPlan.ReleaseID), currentManifest); err == nil || !strings.Contains(err.Error(), "changed since it was activated") {
		t.Fatalf("changed recorded release: %v", err)
	}
	// A malformed or repeating ledger is refused.
	for _, text := range []string{"only two\n", ledgerLine(t, legacyPlan.ReleaseID, legacy) + ledgerLine(t, legacyPlan.ReleaseID, legacy), "\n"} {
		if err := writeFile(ledger, []byte(text), 0644); err != nil {
			t.Fatal(err)
		}
		if _, err := activationRecorded(dir, legacyPlan.ReleaseID, legacyTarget(dir)); err == nil {
			t.Fatalf("accepted ledger %q", text)
		}
	}
	// An empty ledger (a migration that found nothing) records nothing.
	if err := writeFile(ledger, nil, 0644); err != nil {
		t.Fatal(err)
	}
	if recorded, err := activationRecorded(dir, legacyPlan.ReleaseID, legacyTarget(dir)); err != nil || recorded {
		t.Fatalf("empty ledger: %v %v", recorded, err)
	}
}

func dropLine(metadata, prefix string) string {
	var kept []string
	for _, line := range strings.Split(metadata, "\n") {
		if !strings.HasPrefix(line, prefix) {
			kept = append(kept, line)
		}
	}
	return strings.Join(kept, "\n")
}

func replaceValue(metadata, key, value string) string {
	lines := strings.Split(metadata, "\n")
	for i, line := range lines {
		if strings.HasPrefix(line, key+"=") {
			lines[i] = key + "=" + value
		}
	}
	return strings.Join(lines, "\n")
}
