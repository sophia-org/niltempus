package main

import (
	"fmt"
	"os"
	"path/filepath"
)

func installedDesktop() (string, Manifest, error) {
	path, err := filepath.EvalSymlinks(filepath.Join(prefix, "current"))
	if err != nil {
		return "", Manifest{}, err
	}
	m, err := verifyRelease(path)
	return path, m, err
}

func componentInputs(loc Locations, name string) (Config, Source, error) {
	var cfg Config
	if _, err := componentBinary(name); err != nil {
		return cfg, Source{}, err
	}
	if err := readJSON(loc.Config, &cfg); err != nil {
		return cfg, Source{}, err
	}
	source, err := resolveSource(cfg.Repositories[name])
	return cfg, source, err
}

// Only this component's source and dependencies are built. The installed
// desktop provides the compatibility baseline and the already built Nim helper.
func prepareComponent(loc Locations, name string) (ComponentVersion, error) {
	return prepareComponentFor(loc, name, installedDesktop, "")
}

// prepareComponentFor builds and validates one component against the desktop
// release baseline resolves, which need not be installed yet, and stores it
// unselected. kleis is validated as the lock provider under wm, or under the
// selected Hagia when wm is empty.
func prepareComponentFor(loc Locations, name string, baseline func() (string, Manifest, error), wm string) (ComponentVersion, error) {
	binary, err := componentBinary(name)
	if err != nil {
		return ComponentVersion{}, err
	}
	cfg, source, err := componentInputs(loc, name)
	if err != nil {
		return ComponentVersion{}, err
	}
	if source.Signature != "G" {
		return ComponentVersion{}, fmt.Errorf("component source needs a trusted signed commit")
	}
	release, m, err := baseline()
	if err != nil {
		return ComponentVersion{}, err
	}
	if !m.Plan.ComponentUpdates {
		return ComponentVersion{}, fmt.Errorf("install a component-update capable desktop once before preparing components")
	}
	// The Nim products build only from a reviewed dependency closure.
	var nimDeps NimDepsInput
	switch name {
	case "hagia":
		if cfg.Inputs == nil {
			return ComponentVersion{}, fmt.Errorf("Hagia needs reviewed dependency inputs")
		}
		nimDeps = cfg.Inputs.HagiaNimDeps
	case "kleis":
		if cfg.Inputs == nil || cfg.Inputs.KleisNimDeps == nil {
			return ComponentVersion{}, fmt.Errorf("kleis needs reviewed dependency inputs (inputs.kleis_nim_deps)")
		}
		nimDeps = *cfg.Inputs.KleisNimDeps
	}
	if name == "hagia" || name == "kleis" {
		if err := validateNimDeps(name, nimDeps, source.Commit); err != nil {
			return ComponentVersion{}, err
		}
	}
	// Existing SDK contracts remain the compatibility boundary. A different C
	// snapshot needs a desktop release and its cross-component qualification;
	// kleis's lock client therefore waits for a desktop whose SDK has it.
	if usesCSDK(name) {
		sdk, err := committedFile(source, "vendor/sophia-desktop-sdk/manifest.json")
		if err != nil {
			return ComponentVersion{}, err
		}
		sealed, err := os.ReadFile(filepath.Join(release, sealedCSDKManifest))
		if err != nil {
			return ComponentVersion{}, err
		}
		if string(sdk) != string(sealed) {
			return ComponentVersion{}, fmt.Errorf("%s C SDK differs from the installed desktop; prepare a full release for this contract change", name)
		}
	}
	if old, err := componentSelection(loc, name); err == nil && old.Current.Source == source {
		return old.Current, nil
	}
	if err := ownedDirectory(filepath.Join(loc.Cache, "component-builds")); err != nil {
		return ComponentVersion{}, err
	}
	work, err := os.MkdirTemp(filepath.Join(loc.Cache, "component-builds"), name+"-")
	if err != nil {
		return ComponentVersion{}, err
	}
	fmt.Fprintln(os.Stderr, "Component build and logs:", work)
	root, err := buildSource(name, source, loc.Cache, work)
	if err != nil {
		return ComponentVersion{}, err
	}
	if usesCSDK(name) {
		tool := filepath.Join(release, "target/release/sophia-integration-xtask")
		if err := logged(isolated(root, nil, "timeout", "--kill-after=5s", "60s", tool, "verify-c-sdk", filepath.Join(root, "vendor/sophia-desktop-sdk"), "--revision="+m.Plan.Inputs.HagiaCSDKRevision), filepath.Join(work, "verify-sdk.log")); err != nil {
			return ComponentVersion{}, err
		}
	}
	candidate := filepath.Join(root, "target/release", binary)
	switch name {
	case "lom":
		target := filepath.Join(loc.Cache, "lom-target")
		if err := ownedDirectory(target); err != nil {
			return ComponentVersion{}, err
		}
		if err := logged(isolated(root, map[string]string{"CARGO_TARGET_DIR": target}, "timeout", "--kill-after=5s", "1800s", "cargo", "build", "--offline", "--locked", "--release"), filepath.Join(work, "build.log")); err != nil {
			return ComponentVersion{}, err
		}
		candidate = filepath.Join(target, "release", binary)
	case "bemenu":
		jobs, err := buildJobs()
		if err != nil {
			return ComponentVersion{}, err
		}
		if err := logged(isolated(root, nil, "timeout", "--kill-after=5s", "900s", "env", "-u", "MAKEFLAGS", "-u", "MFLAGS", "-u", "MAKELEVEL", "-u", "CFLAGS", "-u", "CPPFLAGS", "-u", "LDFLAGS", "-u", "EXTRA_WARNINGS", "make", "-j"+jobs, "bemenu-sophia", "EXTRA_WARNINGS=-Werror", "GIT_SHA1="+source.Commit, "GIT_TAG="+source.Commit), filepath.Join(work, "build.log")); err != nil {
			return ComponentVersion{}, err
		}
		candidate = filepath.Join(root, binary)
	case "hagia", "kleis":
		build := filepath.Join(work, "build")
		if err := os.Mkdir(build, 0700); err != nil {
			return ComponentVersion{}, err
		}
		deps := filepath.Join(work, "nim-deps.manifest")
		if err := copyFile(nimDeps.Path, deps, 0444); err != nil {
			return ComponentVersion{}, err
		}
		if hash, err := fileDigest(deps); err != nil || hash != nimDeps.SHA256 {
			return ComponentVersion{}, fmt.Errorf("reviewed deps changed while staging")
		}
		artifact := filepath.Join(work, "artifact")
		tool := filepath.Join(release, "target/release/sophia-integration-xtask")
		if err := logged(isolated(root, nil, "timeout", "--kill-after=5s", "1800s", tool, "prepare-product-artifact", name, source.Path, source.Commit, artifact, "--build-dir="+build, "--nim-deps="+deps, "--nim-deps-sha256="+nimDeps.SHA256), filepath.Join(work, "build.log")); err != nil {
			return ComponentVersion{}, err
		}
		candidate = filepath.Join(artifact, binary)
	}
	if head, err := git(root, "rev-parse", "HEAD"); err != nil || head != source.Commit {
		return ComponentVersion{}, fmt.Errorf("component source identity changed")
	}
	if _, err := git(root, "diff", "--exit-code", "HEAD", "--"); err != nil {
		return ComponentVersion{}, err
	}
	// Config validation is done with the installed Sophia. Product builds don't
	// rewrite the user's shell configuration or change its role, GPU or limits.
	// kleis is checked as the profile's lock provider, under the selected WM;
	// a Sophia without the lock provider role refuses it here.
	if name == "hagia" || name == "kleis" {
		profile, err := os.ReadFile(filepath.Join(release, "share/sophia-niltempus-desktop/desktop.kdl"))
		if err != nil {
			return ComponentVersion{}, err
		}
		validationWM := candidate
		render := renderDevelopmentProfile
		if name == "kleis" {
			validationWM = wm
			if validationWM == "" {
				validationWM = componentPath(loc, "hagia")
			}
			render = func(source, binary string) (string, error) {
				return renderLockProviderValidationProfile(source, binary, validationWM)
			}
		}
		rendered, err := render(string(profile), candidate)
		if err != nil {
			return ComponentVersion{}, err
		}
		path := filepath.Join(work, "validation.kdl")
		if err := writeFile(path, []byte(rendered), 0600); err != nil {
			return ComponentVersion{}, err
		}
		if err := preflightProfile(release, work, name, filepath.Join(release, "target/release/sophia"), validationWM, path); err != nil {
			return ComponentVersion{}, err
		}
	}
	hash, err := fileDigest(candidate)
	if err != nil {
		return ComponentVersion{}, err
	}
	v := ComponentVersion{name, source, hash}
	// Keep the candidate unselected until the complete build and validation pass.
	if err := saveComponentVersion(loc, v, candidate); err != nil {
		return ComponentVersion{}, err
	}
	return v, nil
}

// The components that vendor the C desktop SDK, held to the desktop's pin.
func usesCSDK(name string) bool {
	return name == "hagia" || name == "bemenu" || name == "kleis"
}
