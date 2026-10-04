package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
)

// The current executable is a regular file replaced by rename. Existing
// processes keep their inode; Session opens the replacement on its next start.
type ComponentVersion struct {
	Name   string `json:"name"`
	Source Source `json:"source"`
	SHA256 string `json:"sha256"`
}
type ComponentSelection struct {
	Schema   int               `json:"schema"`
	Current  ComponentVersion  `json:"current"`
	Previous *ComponentVersion `json:"previous,omitempty"`
}

func componentBinary(name string) (string, error) {
	switch name {
	case "hagia", "lom", "kleis":
		return name, nil
	case "bemenu":
		return "bemenu-sophia", nil
	}
	return "", fmt.Errorf("unknown component %q; expected hagia, lom, bemenu or kleis", name)
}
func componentPath(loc Locations, name string) string {
	if name == "hagia" {
		return personalHagia(loc)
	}
	return filepath.Join(loc.State, "components", name, "current")
}
func componentDir(loc Locations, name string) string {
	return filepath.Join(loc.State, "components", name)
}
func ownedDirectory(path string) error {
	if err := os.MkdirAll(path, 0700); err != nil {
		return err
	}
	s, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if !s.IsDir() || s.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) || s.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("not a private owned directory: %s", path)
	}
	real, err := filepath.EvalSymlinks(path)
	if err != nil || real != path {
		return fmt.Errorf("directory has symlink components: %s", path)
	}
	return nil
}
func atomicFile(path string, data []byte, mode os.FileMode) error {
	f, err := os.CreateTemp(filepath.Dir(path), ".publish-")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	defer f.Close()
	if _, err = f.Write(data); err != nil {
		return err
	}
	if err = f.Chmod(mode); err != nil {
		return err
	}
	if err = f.Sync(); err != nil {
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}
func componentSelection(loc Locations, name string) (ComponentSelection, error) {
	var s ComponentSelection
	if _, err := componentBinary(name); err != nil {
		return s, err
	}
	if err := readJSON(filepath.Join(componentDir(loc, name), "selection.json"), &s); err != nil {
		return s, err
	}
	valid := func(v ComponentVersion) bool {
		return v.Name == name && lowerHex(v.SHA256, 64) && lowerHex(v.Source.Commit, 40) && v.Source.Signature == "G"
	}
	if s.Schema != 1 || !valid(s.Current) || (s.Previous != nil && !valid(*s.Previous)) {
		return s, fmt.Errorf("invalid %s selection", name)
	}
	info, err := os.Lstat(componentPath(loc, name))
	if err != nil {
		return s, err
	}
	if err = validateRetainedHagia(info); err != nil {
		return s, err
	}
	hash, err := fileDigest(componentPath(loc, name))
	if err != nil || hash != s.Current.SHA256 {
		return s, fmt.Errorf("%s selection and executable differ; refuse restart", name)
	}
	return s, nil
}
func saveComponentVersion(loc Locations, v ComponentVersion, binary string) error {
	if _, err := componentBinary(v.Name); err != nil {
		return err
	}
	if !lowerHex(v.SHA256, 64) || !lowerHex(v.Source.Commit, 40) || v.Source.Signature != "G" {
		return fmt.Errorf("invalid component identity")
	}
	dir := filepath.Join(componentDir(loc, v.Name), "versions")
	if err := ownedDirectory(dir); err != nil {
		return err
	}
	path := filepath.Join(dir, v.SHA256)
	if info, err := os.Lstat(path); err == nil {
		if err := validateRetainedHagia(info); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	data, err := os.ReadFile(binary)
	if err != nil {
		return err
	}
	if digest(data) != v.SHA256 {
		return fmt.Errorf("component changed before publication")
	}
	if old, err := os.ReadFile(path); err == nil {
		if digest(old) != v.SHA256 {
			return fmt.Errorf("retained component was modified")
		}
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	// Publish only a complete version, without replacing an existing version.
	f, err := os.CreateTemp(dir, ".version-")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	defer f.Close()
	if _, err = f.Write(data); err != nil {
		return err
	}
	if err = f.Chmod(0555); err != nil {
		return err
	}
	if err = f.Sync(); err != nil {
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Link(f.Name(), path)
}
func selectComponent(loc Locations, v ComponentVersion, binary string) error {
	if _, err := componentBinary(v.Name); err != nil {
		return err
	}
	dir := componentDir(loc, v.Name)
	if err := ownedDirectory(dir); err != nil {
		return err
	}
	if err := ownedDirectory(filepath.Dir(componentPath(loc, v.Name))); err != nil {
		return err
	}
	var previous *ComponentVersion
	old, err := componentSelection(loc, v.Name)
	if err == nil {
		if old.Current == v {
			return nil
		}
		previous = &old.Current
	} else if !os.IsNotExist(err) {
		return err
	}
	if previous == nil {
		if _, err := os.Lstat(componentPath(loc, v.Name)); err == nil {
			return fmt.Errorf("existing unrecorded component cannot be overwritten")
		} else if !os.IsNotExist(err) {
			return err
		}
	}
	if err := saveComponentVersion(loc, v, binary); err != nil {
		return err
	}
	// Write a recovery record before replacing the executable. A crash between
	// publication steps fails closed; the old immutable binary remains available.
	selection := ComponentSelection{1, v, previous}
	data, err := json.MarshalIndent(selection, "", "  ")
	if err != nil {
		return err
	}
	if err := atomicFile(filepath.Join(dir, "pending.json"), data, 0600); err != nil {
		return err
	}
	bytes, err := os.ReadFile(filepath.Join(dir, "versions", v.SHA256))
	if err != nil {
		return err
	}
	if err := atomicFile(componentPath(loc, v.Name), bytes, 0755); err != nil {
		return err
	}
	if v.Name == "hagia" {
		if err := writeJSON(filepath.Join(filepath.Dir(personalHagia(loc)), "hagia.json"), DevelopmentHagia{v.Source, v.SHA256}); err != nil {
			return err
		}
	}
	return os.Rename(filepath.Join(dir, "pending.json"), filepath.Join(dir, "selection.json"))
}

// Installation initializes missing shell selections but preserves explicit
// component updates. Hagia retains its historical personal path and metadata.
func seedComponents(release string, m Manifest, loc Locations) error {
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		if _, err := componentSelection(loc, name); err == nil {
			continue
		} else if !os.IsNotExist(err) {
			return err
		}
		binary, _ := componentBinary(name)
		src := filepath.Join(release, "target/release", binary)
		v := ComponentVersion{name, m.Plan.Sources[name], m.Files["target/release/"+binary].SHA256}
		if name == "hagia" {
			var personal DevelopmentHagia
			if err := readJSON(filepath.Join(filepath.Dir(personalHagia(loc)), "hagia.json"), &personal); err != nil {
				return fmt.Errorf("cannot bind personal Hagia: %w", err)
			}
			v.Source, v.SHA256, src = personal.Source, personal.SHA256, personalHagia(loc)
		}
		// Seed by recording the existing WM without replacing its inode or metadata.
		if name == "hagia" {
			if err := saveComponentVersion(loc, v, src); err != nil {
				return err
			}
			data, _ := json.MarshalIndent(ComponentSelection{Schema: 1, Current: v}, "", "  ")
			if err := atomicFile(filepath.Join(componentDir(loc, name), "selection.json"), data, 0600); err != nil {
				return err
			}
		} else if err := selectComponent(loc, v, src); err != nil {
			return err
		}
	}
	return nil
}

func rollbackComponent(loc Locations, name string) error {
	s, err := componentSelection(loc, name)
	if err != nil {
		return err
	}
	if s.Previous == nil {
		return fmt.Errorf("%s has no previous component selection", name)
	}
	return selectComponent(loc, *s.Previous, filepath.Join(componentDir(loc, name), "versions", s.Previous.SHA256))
}
