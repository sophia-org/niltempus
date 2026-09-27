package main

import (
	"fmt"
	"os"
	"path/filepath"
	"syscall"
)

const developmentDesktopFile = "sophia-niltempus-desktop-hagia-development.desktop"

type DevelopmentHagia struct {
	Source Source `json:"source"`
	SHA256 string `json:"sha256"`
}

func prepareHagia(loc Locations) (string, error) {
	source, err := developmentSource(loc)
	if err != nil {
		return "", err
	}
	release, err := filepath.EvalSymlinks(filepath.Join(prefix, "current"))
	if err != nil {
		return "", err
	}
	if _, err := verifyRelease(release); err != nil {
		return "", err
	}
	dir := filepath.Join(loc.State, "development")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return "", err
	}
	binary := filepath.Join(dir, "hagia")
	metadata := filepath.Join(dir, "hagia.json")
	var previous DevelopmentHagia
	hash, hashErr := fileDigest(binary)
	reuse := readJSON(metadata, &previous) == nil && hashErr == nil && previous.Source == source && previous.SHA256 == hash
	work, err := os.MkdirTemp(dir, "prepare-")
	if err != nil {
		return "", err
	}
	candidate := binary
	if !reuse {
		root := filepath.Join(work, "hagia-source")
		if err := checked(buildCommand("git", "clone", "--quiet", "--shared", "--no-checkout", source.Path, root)); err != nil {
			return "", err
		}
		if _, err := git(root, "checkout", "--quiet", "--detach", source.Commit); err != nil {
			return "", err
		}
		candidate = filepath.Join(work, "hagia")
		if err := logged(isolated(root, nil, "nim", "c", "-d:release", "--hints:off", "--path:src", "--nimcache:"+filepath.Join(loc.Cache, "hagia-nim"), "-o:"+candidate, "src/hagia.nim"), filepath.Join(work, "build.log")); err != nil {
			return "", err
		}
	}
	// Validate the new WM against the already installed Sophia and profile.
	// No Sophia, Lom or shell build is involved.
	profile := filepath.Join(release, "share/sophia-niltempus-desktop/desktop.kdl")
	installedProfile, err := os.ReadFile(profile)
	if err != nil {
		return "", err
	}
	validation, err := renderDevelopmentProfile(string(installedProfile), candidate)
	if err != nil {
		return "", err
	}
	// The offline checker admits user-owned files. Copy the sealed profile and
	// apply the same WM override as the normal session launcher.
	validationPath := filepath.Join(work, "validation.kdl")
	if err := writeFile(validationPath, []byte(validation), 0600); err != nil {
		return "", err
	}
	if err := preflightProfile(release, work, "development", filepath.Join(release, "target/release/sophia"), candidate, validationPath); err != nil {
		return "", err
	}
	if !reuse {
		hash, err = fileDigest(candidate)
		if err != nil {
			return "", err
		}
		if err := publishPersonalHagia(candidate, source, hash, loc); err != nil {
			return "", err
		}
	}
	return binary, nil
}

// Explicit WM preparation uses the installed release for compatibility checks.
// It must not depend on the next desktop's integration pin or provisioning.
func developmentSource(loc Locations) (Source, error) {
	var config Config
	if err := readJSON(loc.Config, &config); err != nil {
		return Source{}, err
	}
	return resolveSource(config.Repositories["hagia"])
}

// Keep the historical path stable; the normal login now uses it too.
func personalHagia(loc Locations) string {
	return filepath.Join(loc.State, "development", "hagia")
}

func installPersonalHagia(artifact string, manifest Manifest, loc Locations) error {
	return publishPersonalHagiaMode(filepath.Join(artifact, "target/release/hagia"), manifest.Plan.Sources["hagia"], manifest.Files["target/release/hagia"].SHA256, loc, true)
}

func publishPersonalHagia(candidate string, source Source, expected string, loc Locations) error {
	return publishPersonalHagiaMode(candidate, source, expected, loc, false)
}

func validateRetainedHagia(info os.FileInfo) error {
	if !info.Mode().IsRegular() || info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) || info.Mode().Perm()&0100 == 0 || info.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("personal Hagia must be an owned executable regular file, not writable by others")
	}
	return nil
}

// Installing or rolling back a desktop may initialize an absent personal WM,
// but only explicit preparation may replace one the user already selected.
func publishPersonalHagiaMode(candidate string, source Source, expected string, loc Locations, absentOnly bool) error {
	binary := personalHagia(loc)
	dir := filepath.Dir(binary)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	info, err := os.Lstat(dir)
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) || info.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("personal WM directory must be owned and not writable by others: %s", dir)
	}
	current, err := os.Lstat(binary)
	if err == nil && (!current.Mode().IsRegular() || current.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid())) {
		return fmt.Errorf("personal Hagia must be an owned regular file")
	}
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	if absentOnly && current != nil {
		return validateRetainedHagia(current)
	}
	data, err := os.ReadFile(candidate)
	if err != nil {
		return err
	}
	if digest(data) != expected {
		return fmt.Errorf("prepared Hagia changed before publication")
	}
	hash, _ := fileDigest(binary)
	if current == nil || hash != expected || current.Mode().Perm() != 0755 {
		staged, err := os.CreateTemp(dir, ".hagia-")
		if err != nil {
			return err
		}
		defer os.Remove(staged.Name())
		defer staged.Close()
		if _, err := staged.Write(data); err != nil {
			return err
		}
		if err := staged.Chmod(0755); err != nil {
			return err
		}
		if err := staged.Sync(); err != nil {
			return err
		}
		if err := staged.Close(); err != nil {
			return err
		}
		if absentOnly {
			// No replacement, even if a user publishes a WM after the lookup.
			if err := os.Link(staged.Name(), binary); os.IsExist(err) {
				retained, err := os.Lstat(binary)
				if err != nil {
					return err
				}
				return validateRetainedHagia(retained)
			} else if err != nil {
				return err
			}
		} else {
			// Running processes retain their inode; no live process is signalled.
			if err := os.Rename(staged.Name(), binary); err != nil {
				return err
			}
		}
	}
	return writeJSON(filepath.Join(dir, "hagia.json"), DevelopmentHagia{source, expected})
}
