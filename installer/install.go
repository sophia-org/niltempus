package main

import (
	"fmt"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
)

func privileged(script, target string) *exec.Cmd {
	return exec.Command("sudo", "env", "SOPHIA_INSTALL_PREFIX="+prefix, "SOPHIA_COMMAND_DIR="+prefix+"/bin", "SOPHIA_SESSION_DIR="+prefix+"/sessions", "SOPHIA_INSTALL_PROOF_SESSIONS=0", script, target)
}

func installRelease(artifact string, loc Locations) error {
	artifact, err := filepath.Abs(artifact)
	if err != nil {
		return err
	}
	artifact, err = filepath.EvalSymlinks(artifact)
	if err != nil {
		return err
	}
	manifest, err := verifyRelease(artifact)
	if err != nil {
		return err
	}
	target := filepath.Join(prefix, "releases", manifest.Plan.ReleaseID)
	if _, err := os.Stat(target); err == nil {
		existing, err := verifyRelease(target)
		if err != nil {
			return err
		}
		if !maps.Equal(existing.Files, manifest.Files) {
			return fmt.Errorf("installed release has same identity but different files")
		}
		if err := checked(privileged(filepath.Join(target, "tools/activate_live_session_release.sh"), target)); err != nil {
			return err
		}
	} else if os.IsNotExist(err) {
		if manifest.Plan.Schema != 2 {
			return fmt.Errorf("new installations require release schema 2; schema 1 is supported only for an already installed release")
		}
		if err := checked(privileged(filepath.Join(artifact, "tools/install_live_session.sh"), artifact)); err != nil {
			return err
		}
	} else {
		return err
	}
	if err := installPersonalHagia(artifact, manifest, loc); err != nil {
		return err
	}
	// Sophia's generic entries stay under our prefix. Only this personal login
	// entry is registered with the display manager.
	if err := checked(exec.Command("sudo", "install", "-m", "644", filepath.Join(target, "share/wayland-sessions", desktopFile), filepath.Join("/usr/share/wayland-sessions", desktopFile))); err != nil {
		return err
	}
	if err := checked(exec.Command("sudo", "rm", "-f", "--", filepath.Join("/usr/share/wayland-sessions", developmentDesktopFile))); err != nil {
		return err
	}
	// The current-IPC rollback entry exists only while the selected release
	// carries its launcher: rolling back to an older release removes it
	// instead of leaving it dangling. The retired sealed 9P entry always goes.
	if err := checked(exec.Command("sudo", "rm", "-f", "--", filepath.Join("/usr/share/wayland-sessions", retiredNineDesktopFile))); err != nil {
		return err
	}
	ipc := filepath.Join(target, "share/wayland-sessions", ipcDesktopFile)
	if _, err := os.Stat(ipc); err == nil {
		if err := checked(exec.Command("sudo", "install", "-m", "644", ipc, filepath.Join("/usr/share/wayland-sessions", ipcDesktopFile))); err != nil {
			return err
		}
	} else if os.IsNotExist(err) {
		if err := checked(exec.Command("sudo", "rm", "-f", "--", filepath.Join("/usr/share/wayland-sessions", ipcDesktopFile))); err != nil {
			return err
		}
	} else {
		return err
	}
	if _, err := verifyRelease(filepath.Join(prefix, "current")); err != nil {
		return err
	}
	fmt.Printf("Installed %s. Select 'Sophia niltempus Desktop' at your next login. Running sessions are unchanged.\n", manifest.Plan.ReleaseID)
	return nil
}

func rollback(loc Locations) error {
	previous, err := filepath.EvalSymlinks(filepath.Join(prefix, "previous"))
	if err != nil {
		return err
	}
	if filepath.Dir(previous) != filepath.Join(prefix, "releases") {
		return fmt.Errorf("previous release is outside the desktop prefix")
	}
	if err := installRelease(previous, loc); err != nil {
		return err
	}
	fmt.Printf("Selected %s for the next login. Running sessions are unchanged.\n", previous)
	return nil
}
