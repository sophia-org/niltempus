package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
)

const sophiaRepositoryURL = "https://github.com/sophia-org/sophia.git"

// Old integration bindings remain readable for rollback. A release can never
// name two competing tooling sources, even if their fields happen to match.
func toolingBinding(plan Plan) (*IntegrationPlan, error) {
	if plan.Niltempus != nil && plan.Integration != nil {
		return nil, fmt.Errorf("release names both niltempus and historical integration sources")
	}
	if plan.Niltempus != nil {
		return plan.Niltempus, nil
	}
	return plan.Integration, nil
}

// This is an explicit provisioning input, never a reason to fetch during build.
// Pin and lock bytes come from the selected commit, not the user's worktree.
func planIntegration(repo Repository, sophia Source) (IntegrationPlan, error) {
	source, err := resolveSource(repo)
	if err != nil {
		return IntegrationPlan{}, err
	}
	if source.Signature != "G" {
		return IntegrationPlan{}, fmt.Errorf("the external packager requires a trusted signed commit")
	}
	marker, err := os.ReadFile(filepath.Join(source.Path, ".provision/accepted"))
	if err != nil {
		return IntegrationPlan{}, fmt.Errorf("provision the selected integration revision explicitly first: %w", err)
	}
	pin, err := committedFile(source, "pins/sophia.toml")
	if err != nil {
		return IntegrationPlan{}, err
	}
	lock, err := committedFile(source, "Cargo.lock")
	if err != nil {
		return IntegrationPlan{}, err
	}
	home, err := validateIntegrationProvision(pin, lock, marker, sophia.Commit)
	if err != nil {
		return IntegrationPlan{}, err
	}
	if err := integrationHome(home, source.Path, sophia.Path); err != nil {
		return IntegrationPlan{}, err
	}
	return IntegrationPlan{source, home, digest(lock)}, nil
}

func committedFile(source Source, name string) ([]byte, error) {
	// Keep the final newline: Cargo.lock's byte digest is part of the binding.
	cmd := buildCommand("git", "-C", source.Path, "show", source.Commit+":"+name)
	data, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("read committed %s: %w", name, err)
	}
	return data, nil
}

func exactFields(data []byte, names []string, quoted bool) (map[string]string, error) {
	fields := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || (quoted && strings.HasPrefix(line, "#")) {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		key, value = strings.TrimSpace(key), strings.TrimSpace(value)
		if !ok || fields[key] != "" || value == "" {
			return nil, fmt.Errorf("invalid or repeated integration field %q", key)
		}
		if quoted {
			var err error
			value, err = strconv.Unquote(value)
			if err != nil || value == "" {
				return nil, fmt.Errorf("invalid integration pin value for %s", key)
			}
		}
		fields[key] = value
	}
	if len(fields) != len(names) {
		return nil, fmt.Errorf("unexpected integration fields")
	}
	for _, name := range names {
		if fields[name] == "" {
			return nil, fmt.Errorf("missing integration field %s", name)
		}
	}
	return fields, nil
}

func validateIntegrationProvision(pin, lock, marker []byte, sophiaCommit string) (string, error) {
	p, err := exactFields(pin, []string{"url", "rev"}, true)
	if err != nil {
		return "", err
	}
	m, err := exactFields(marker, []string{"url", "rev", "cargo_lock_sha256", "cargo_home"}, false)
	if err != nil {
		return "", err
	}
	if p["url"] != sophiaRepositoryURL || m["url"] != p["url"] || p["rev"] != sophiaCommit || m["rev"] != p["rev"] {
		return "", fmt.Errorf("integration pin and provisioning must match the selected Sophia commit and canonical URL")
	}
	if m["cargo_lock_sha256"] != digest(lock) {
		return "", fmt.Errorf("integration provisioning does not match the committed Cargo.lock")
	}
	if !filepath.IsAbs(m["cargo_home"]) || filepath.Clean(m["cargo_home"]) != m["cargo_home"] {
		return "", fmt.Errorf("integration Cargo home must be an absolute clean path")
	}
	return m["cargo_home"], nil
}

func integrationHome(home string, sources ...string) error {
	real, err := filepath.EvalSymlinks(home)
	if err != nil || real != home {
		return fmt.Errorf("integration Cargo home must exist without symlink components: %s", home)
	}
	info, err := os.Stat(home)
	if err != nil || !info.IsDir() || info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) || info.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("integration Cargo home must be an owned directory, not writable by others: %s", home)
	}
	for _, source := range sources {
		if home == source || strings.HasPrefix(home, source+string(os.PathSeparator)) || strings.HasPrefix(source, home+string(os.PathSeparator)) {
			return fmt.Errorf("integration Cargo home must be outside source trees: %s", home)
		}
	}
	return nil
}

// Revalidate the accepted input before writing the private clone's marker. This
// carries an existing acceptance, not a new provisioning decision.
func carryIntegrationProvision(plan Plan, root string) error {
	bound, err := toolingBinding(plan)
	if err != nil {
		return err
	}
	if bound == nil || plan.Niltempus == nil {
		return fmt.Errorf("new builds require the single niltempus installer and tooling source")
	}
	again, err := planIntegration(Repository{bound.Source.Path, bound.Source.Commit}, plan.Sources["sophia"])
	if err != nil {
		return err
	}
	if again.Source.Commit != bound.Source.Commit || again.CargoHome != bound.CargoHome || again.CargoLockSHA256 != bound.CargoLockSHA256 {
		return fmt.Errorf("integration provisioning changed after planning")
	}
	if err := integrationHome(bound.CargoHome, root); err != nil {
		return err
	}
	marker := fmt.Sprintf("url=%s\nrev=%s\ncargo_lock_sha256=%s\ncargo_home=%s\n", sophiaRepositoryURL, plan.Sources["sophia"].Commit, bound.CargoLockSHA256, bound.CargoHome)
	return writeFile(filepath.Join(root, ".provision/accepted"), []byte(marker), 0600)
}

func packageDesktop(plan Plan, roots map[string]string, work, cache string) (string, error) {
	root := roots["integration"]
	if err := carryIntegrationProvision(plan, root); err != nil {
		return "", err
	}
	bootstrap, buildDir, err := packageBuildTargets(cache, work)
	if err != nil {
		return "", err
	}
	env := map[string]string{"CARGO_HOME": plan.Niltempus.CargoHome, "CARGO_TARGET_DIR": bootstrap}
	if err := logged(isolated(root, env, "cargo", "build", "--offline", "--locked", "-p", "xtask", "--bin", "xtask"), filepath.Join(work, "build-integration.log")); err != nil {
		return "", err
	}
	tool := filepath.Join(env["CARGO_TARGET_DIR"], "debug/xtask")
	for _, check := range []string{"check-pins", "check-provision"} {
		if err := logged(isolated(root, env, tool, check), filepath.Join(work, check+".log")); err != nil {
			return "", err
		}
	}
	if plan.Schema != currentPlanSchema {
		return "", fmt.Errorf("new builds require plan schema %d", currentPlanSchema)
	}
	if err := validateInputs(plan.Inputs, plan.Sources); err != nil {
		return "", err
	}
	hagiaDeps, narthexDeps, err := stageInputs(plan.Inputs, work)
	if err != nil {
		return "", err
	}
	pairBuild := filepath.Join(work, "pair-build")
	if err := os.Mkdir(pairBuild, 0700); err != nil {
		return "", err
	}
	pair := filepath.Join(work, "wm-pair")
	if err := logged(isolated(root, env, prepareWMPairArgs(tool, plan, roots, pair, pairBuild, hagiaDeps, narthexDeps)...), filepath.Join(work, "prepare-wm-pair.log")); err != nil {
		return "", err
	}
	var hashes [3]string
	for i, name := range []string{"hagia", "narthex", "default.kdl"} {
		path := filepath.Join(pair, name)
		info, err := os.Lstat(path)
		if err != nil || !info.Mode().IsRegular() {
			return "", fmt.Errorf("prepared WM pair has no regular %s", name)
		}
		hash, err := fileDigest(path)
		if err != nil {
			return "", err
		}
		hashes[i] = hash
	}
	stage := filepath.Join(work, "package")
	if err := logged(isolated(root, env, packageDesktopArgs(tool, plan, roots, pair, hashes, buildDir, stage)...), filepath.Join(work, "package-desktop.log")); err != nil {
		return "", err
	}
	// Use the just-built, source-bound verifier before adding the personal
	// desktop files. Never execute a verifier supplied by the candidate.
	if err := logged(isolated(root, env, tool, "verify-release", stage, "--c-sdk-rev="+plan.Inputs.HagiaCSDKRevision), filepath.Join(work, "verify-package.log")); err != nil {
		return "", err
	}
	return stage, nil
}

func verifyIntegrationRelease(metadata []byte, plan Plan, files map[string]FileRecord, root string) error {
	bound, err := toolingBinding(plan)
	if err != nil {
		return err
	}
	if bound == nil {
		return fmt.Errorf("release has no desktop tooling binding")
	}
	schema := legacyExternalSchema
	if plan.Schema >= currentPlanSchema {
		// The current verifier: external schema 7 with Hagia's vendored C SDK,
		// never delegated to the release's bundled verifier.
		if plan.Niltempus == nil || plan.Inputs == nil {
			return fmt.Errorf("plan schema %d requires the niltempus binding and explicit inputs", plan.Schema)
		}
		if err := verifyExternalSchema7(metadata, root, files, plan.Inputs.HagiaCSDKRevision); err != nil {
			return err
		}
		schema = currentExternalSchema
	}
	expected := map[string]string{
		"schema":                       schema,
		"commit":                       plan.Sources["sophia"].Commit,
		"integration_commit":           bound.Source.Commit,
		"hagia_source_commit":          plan.Sources["hagia"].Commit,
		"narthex_source_commit":        plan.Sources["narthex"].Commit,
		"hagia_binary_sha256":          files["target/release/hagia"].SHA256,
		"hagia_shell_binary_sha256":    files["target/release/narthex"].SHA256,
		"hagia_default_profile_sha256": files["share/sophia-policy/hagia/default.kdl"].SHA256,
	}
	for key, value := range expected {
		if value == "" {
			return fmt.Errorf("incomplete desktop integration binding: %s", key)
		}
	}
	for _, line := range strings.Split(string(metadata), "\n") {
		key, value, ok := strings.Cut(line, "=")
		if want, required := expected[key]; required {
			if !ok || want == "" || value != want {
				return fmt.Errorf("packaged %s differs from the desktop plan or sealed files", key)
			}
			// An empty expectation makes a duplicate fail too.
			expected[key] = ""
		}
	}
	for key, remaining := range expected {
		if remaining != "" {
			return fmt.Errorf("package manifest lacks %s", key)
		}
	}
	return nil
}
