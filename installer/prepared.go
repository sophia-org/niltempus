package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// Selection is explicit: directory timestamps never choose a desktop. The
// manifest digest also refuses a selected release that was later re-sealed.
type PreparedRelease struct {
	Schema         int    `json:"schema"`
	Directory      string `json:"directory"`
	ReleaseID      string `json:"release_id"`
	ManifestSHA256 string `json:"manifest_sha256"`
}

func preparedIdentity(directory string) (PreparedRelease, error) {
	root, err := filepath.Abs(directory)
	if err != nil {
		return PreparedRelease{}, err
	}
	root, err = filepath.EvalSymlinks(root)
	if err != nil {
		return PreparedRelease{}, err
	}
	path := filepath.Join(root, "desktop-manifest.json")
	before, err := fileDigest(path)
	if err != nil {
		return PreparedRelease{}, err
	}
	manifest, err := verifyRelease(root)
	if err != nil {
		return PreparedRelease{}, err
	}
	if manifest.Plan.Schema != 2 {
		return PreparedRelease{}, fmt.Errorf("preparing a new installation requires release schema 2")
	}
	after, err := fileDigest(path)
	if err != nil {
		return PreparedRelease{}, err
	}
	if before != after {
		return PreparedRelease{}, fmt.Errorf("release manifest changed during verification")
	}
	return PreparedRelease{1, root, manifest.Plan.ReleaseID, before}, nil
}

func selectPreparedRelease(directory string, loc Locations) error {
	selected, err := preparedIdentity(directory)
	if err != nil {
		return err
	}
	data, err := json.MarshalIndent(selected, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(loc.State, 0700); err != nil {
		return err
	}
	file, err := os.CreateTemp(loc.State, ".prepared-")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	defer file.Close()
	if _, err := file.Write(append(data, '\n')); err != nil {
		return err
	}
	if err := file.Sync(); err != nil {
		return err
	}
	if err := file.Close(); err != nil {
		return err
	}
	return os.Rename(file.Name(), filepath.Join(loc.State, "prepared.json"))
}

func preparedRelease(loc Locations) (string, error) {
	var selected PreparedRelease
	if err := readJSON(filepath.Join(loc.State, "prepared.json"), &selected); err != nil {
		return "", err
	}
	if selected.Schema != 1 || !filepath.IsAbs(selected.Directory) {
		return "", fmt.Errorf("invalid prepared release selection")
	}
	actual, err := preparedIdentity(selected.Directory)
	if err != nil {
		return "", fmt.Errorf("prepared release: %w", err)
	}
	if actual != selected {
		return "", fmt.Errorf("prepared release changed; build or prepare it again explicitly")
	}
	return actual.Directory, nil
}
