package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
)

// A saved copy of the mutable component state. Retained versions are never
// removed. Recovery includes the failing publisher, which may already have
// replaced its executable before its selection record failed to publish.
type componentRestoreFile struct {
	Name string      `json:"component"`
	Path string      `json:"path"`
	Copy string      `json:"copy,omitempty"`
	Mode os.FileMode `json:"mode"`
}
type componentRestorePoint struct {
	Directory string
	Files     []componentRestoreFile
}

func snapshotComponents(loc Locations, names []string) (componentRestorePoint, error) {
	var point componentRestorePoint
	// Refuse all interrupted or incomplete states before any preparation or sudo.
	for _, name := range names {
		paths := componentMutablePaths(loc, name)
		absent := true
		for _, path := range paths {
			info, err := os.Lstat(path)
			if os.IsNotExist(err) {
				continue
			}
			if err != nil {
				return point, err
			}
			if !info.Mode().IsRegular() {
				return point, fmt.Errorf("not a regular component file: %s", path)
			}
			if filepath.Base(path) == "pending.json" {
				return point, fmt.Errorf("%s has an interrupted publication; recover its selection first", name)
			}
			absent = false
		}
		if !absent {
			if _, err := componentSelection(loc, name); err != nil {
				return point, err
			}
		}
	}
	parent := filepath.Join(loc.State, "install-recovery")
	if err := ownedDirectory(parent); err != nil {
		return point, err
	}
	dir, err := os.MkdirTemp(parent, "components-")
	if err != nil {
		return point, err
	}
	point.Directory = dir
	for _, name := range names {
		for _, path := range componentMutablePaths(loc, name) {
			saved := componentRestoreFile{Name: name, Path: path}
			info, err := os.Lstat(path)
			if err == nil {
				saved.Mode = info.Mode().Perm()
				saved.Copy = filepath.Join(dir, fmt.Sprint(len(point.Files)))
				if err := copyFile(path, saved.Copy, 0600); err != nil {
					return point, err
				}
			} else if !os.IsNotExist(err) {
				return point, err
			}
			point.Files = append(point.Files, saved)
		}
	}
	return point, writeJSON(filepath.Join(dir, "files.json"), point.Files)
}

func componentMutablePaths(loc Locations, name string) []string {
	paths := []string{componentPath(loc, name)}
	if name == "hagia" {
		paths = append(paths, filepath.Join(filepath.Dir(personalHagia(loc)), "hagia.json"))
	}
	// Restore executable and metadata before the selection; clear pending last.
	return append(paths, filepath.Join(componentDir(loc, name), "selection.json"), filepath.Join(componentDir(loc, name), "pending.json"))
}

func (p componentRestorePoint) restore(name string) error {
	for _, file := range p.Files {
		if file.Name != name {
			continue
		}
		if file.Copy == "" {
			if err := os.Remove(file.Path); err != nil && !os.IsNotExist(err) {
				return err
			}
			continue
		}
		data, err := os.ReadFile(file.Copy)
		if err != nil {
			return err
		}
		// Leave unchanged files (and their running inodes) alone.
		old, err := os.ReadFile(file.Path)
		info, statErr := os.Lstat(file.Path)
		if err == nil && statErr == nil && info.Mode().IsRegular() && info.Mode().Perm() == file.Mode && bytes.Equal(old, data) {
			continue
		}
		if err := ownedDirectory(filepath.Dir(file.Path)); err != nil {
			return err
		}
		if err := atomicFile(file.Path, data, file.Mode); err != nil {
			return err
		}
	}
	return nil
}
