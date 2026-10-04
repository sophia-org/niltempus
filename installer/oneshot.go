package main

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
)

// The components one installation prepares, in order. Hagia comes first so
// that kleis is validated as the lock provider under the Hagia it will run with.
var oneShotOrder = []string{"hagia", "lom", "bemenu", "kleis"}

// oneShot installs a desktop release together with the configured
// components. Every component is built and validated against that release
// before anything is installed, so a preparation failure leaves the installed
// desktop and every selection as they were. After installation the prepared components
// are selected; if one cannot be, the attempted selections are restored exactly.
type oneShot struct {
	prepare     func(name, wm string) (ComponentVersion, error)
	versionPath func(ComponentVersion) string
	install     func() error
	selected    func(name string) (ComponentSelection, error)
	choose      func(ComponentVersion) error
	restore     func(name string) error
	validate    func() error
	preflight   func([]ComponentVersion) error
}

func (o oneShot) run(configured map[string]Repository) ([]ComponentVersion, error) {
	var prepared []ComponentVersion
	wm := ""
	for _, name := range oneShotOrder {
		if _, ok := configured[name]; !ok {
			continue
		}
		v, err := o.prepare(name, wm)
		if err != nil {
			return nil, fmt.Errorf("%s was not prepared: %w; nothing was installed or selected", name, err)
		}
		if name == "hagia" {
			wm = o.versionPath(v)
		}
		prepared = append(prepared, v)
	}
	if o.preflight != nil {
		if err := o.preflight(prepared); err != nil {
			return nil, fmt.Errorf("prepared login profile refused: %w; nothing was installed or selected", err)
		}
	}
	if err := o.install(); err != nil {
		names := []string{}
		for _, v := range prepared {
			names = append(names, v.Name)
		}
		return nil, o.recover(names, fmt.Errorf("installation failed: %w; desktop activation may already have occurred", err))
	}
	var switched []string
	for _, v := range prepared {
		if old, err := o.selected(v.Name); err == nil && old.Current == v {
			continue
		}
		switched = append(switched, v.Name)
		if err := o.choose(v); err != nil {
			return nil, o.recover(switched, fmt.Errorf("selecting %s failed: %w; the new desktop stays installed", v.Name, err))
		}
	}
	if o.validate != nil {
		if err := o.validate(); err != nil {
			return nil, o.recover(switched, fmt.Errorf("login profile failed: %w; the new desktop stays installed", err))
		}
	}
	return prepared, nil
}

func (o oneShot) recover(names []string, cause error) error {
	var failures []error
	for i := len(names) - 1; i >= 0; i-- {
		if err := o.restore(names[i]); err != nil {
			failures = append(failures, fmt.Errorf("%s: %w", names[i], err))
		}
	}
	if len(failures) > 0 {
		return fmt.Errorf("%w; component restoration also failed: %w", cause, errors.Join(failures...))
	}
	return fmt.Errorf("%w; restored original component state for %v", cause, names)
}

// installWithComponents is `niltempus install`: the release at artifact and
// the components the configuration names, in one step.
func installWithComponents(artifact string, loc Locations) (result error) {
	m, err := verifyRelease(artifact)
	if err != nil {
		return err
	}
	var cfg Config
	if err := readJSON(loc.Config, &cfg); err != nil {
		return err
	}
	names := []string{}
	for _, name := range oneShotOrder {
		if _, ok := cfg.Repositories[name]; ok {
			names = append(names, name)
		}
	}
	profile, err := os.ReadFile(filepath.Join(artifact, "share/sophia-niltempus-desktop/desktop.kdl"))
	if err != nil {
		return err
	}
	rendered, err := renderComponentProfile(string(profile), loc)
	if err != nil {
		return err
	}
	if strings.Contains(rendered, componentPath(loc, "kleis")) {
		if _, ok := cfg.Repositories["kleis"]; !ok {
			return fmt.Errorf("the release needs a configured kleis repository")
		}
	}
	backup, err := snapshotComponents(loc, names)
	if err != nil {
		return err
	}
	defer func() {
		if result != nil {
			fmt.Fprintln(os.Stderr, "Original component state and preparation logs:", backup.Directory)
		} else if err := os.RemoveAll(backup.Directory); err != nil {
			fmt.Fprintln(os.Stderr, "Could not remove successful-install backup:", backup.Directory, err)
		}
	}()
	_, previous, previousErr := installedDesktop()
	sealedSDK, err := os.ReadFile(filepath.Join(artifact, sealedCSDKManifest))
	if err != nil {
		return err
	}
	baseline := func() (string, Manifest, error) { return artifact, m, nil }
	versionPath := func(v ComponentVersion) string {
		return filepath.Join(componentDir(loc, v.Name), "versions", v.SHA256)
	}
	o := oneShot{
		prepare: func(name, wm string) (ComponentVersion, error) {
			if previousErr == nil && unchangedPackagedComponent(name, previous.Plan, m.Plan) {
				if selected, err := componentSelection(loc, name); err == nil && retainedComponentCompatible(name, selected.Current.Source, sealedSDK, committedFile) {
					return selected.Current, nil
				}
			}
			return prepareComponentFor(loc, name, baseline, wm)
		},
		versionPath: versionPath,
		install: func() error {
			var currentConfig Config
			if err := readJSON(loc.Config, &currentConfig); err != nil {
				return err
			}
			if !reflect.DeepEqual(cfg, currentConfig) {
				return fmt.Errorf("configuration changed during component preparation; nothing was activated")
			}
			if err := preparedMatchesConfiguration(artifact, loc); err != nil {
				return err
			}
			return installRelease(artifact, loc)
		},
		selected: func(name string) (ComponentSelection, error) { return componentSelection(loc, name) },
		choose:   func(v ComponentVersion) error { return selectComponent(loc, v, versionPath(v)) },
		restore:  backup.restore,
		validate: func() error { _, err := componentProfile(artifact, loc); return err },
		preflight: func(versions []ComponentVersion) error {
			paths := map[string]string{}
			for _, v := range versions {
				paths[v.Name] = versionPath(v)
			}
			rendered, err := renderComponentProfileWith(string(profile), func(name string) string { return paths[name] })
			if err != nil {
				return err
			}
			path := filepath.Join(backup.Directory, "prepared-profile.kdl")
			if err := writeFile(path, []byte(rendered), 0600); err != nil {
				return err
			}
			return preflightProfile(artifact, backup.Directory, "prepared", filepath.Join(artifact, "target/release/sophia"), paths["hagia"], path)
		},
	}
	prepared, err := o.run(cfg.Repositories)
	if err != nil {
		_, current, currentErr := installedDesktop()
		if currentErr == nil && current.Plan.ReleaseID == m.Plan.ReleaseID {
			if previousErr == nil && previous.Plan.ReleaseID != m.Plan.ReleaseID {
				return fmt.Errorf("%w; before logging in, run niltempus rollback to return to %s", err, previous.Plan.ReleaseID)
			}
			return fmt.Errorf("%w; do not log in until the reported component failure is repaired", err)
		}
		return err
	}
	for _, v := range prepared {
		fmt.Printf("Selected %s %s\n", v.Name, v.Source.Commit)
	}
	fmt.Println("Log out and log in to use the new desktop.")
	return nil
}

// Preserve independent personal updates when their packaged source did not
// change. C clients additionally need the new desktop's exact SDK snapshot.
func unchangedPackagedComponent(name string, before, after Plan) bool {
	old, had := before.Sources[name]
	next, have := after.Sources[name]
	return had && have && old.Commit == next.Commit && old.Path == next.Path
}

func retainedComponentCompatible(name string, source Source, sealed []byte, read func(Source, string) ([]byte, error)) bool {
	if !usesCSDK(name) {
		return true
	}
	manifest, err := read(source, "vendor/sophia-desktop-sdk/manifest.json")
	return err == nil && bytes.Equal(manifest, sealed)
}

// preparedMatchesConfiguration refuses to install a prepared release that the
// current configuration would not build, instead of installing it silently.
func preparedMatchesConfiguration(path string, loc Locations) error {
	m, err := verifyRelease(path)
	if err != nil {
		return err
	}
	plan, err := createPlan(loc)
	if err != nil {
		return fmt.Errorf("cannot confirm that the prepared release matches the configuration: %w", err)
	}
	if m.Plan.ReleaseID != plan.ReleaseID {
		return fmt.Errorf("the prepared release %s does not match the configuration, which plans %s; run niltempus build, or prepare the release built for this configuration", m.Plan.ReleaseID, plan.ReleaseID)
	}
	return nil
}
