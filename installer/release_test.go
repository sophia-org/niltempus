package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func fixtureRelease(t *testing.T) (string, Plan) {
	t.Helper()
	root := t.TempDir()
	plan := Plan{Schema: 2, ProfileSHA256: digest([]byte("profile")), InstallerSHA256: digest([]byte("installer"))}
	plan.ReleaseID = releaseID(plan)
	for _, name := range []string{"sophia", "hagia", "narthex", "lom", "bemenu-sophia"} {
		if err := writeFile(filepath.Join(root, "target/release", name), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	for _, path := range []string{"bin/sophia-niltempus-desktop-session", ipcLauncher, "bin/sophia-hagia-session", "tools/install_live_session.sh", "tools/activate_live_session_release.sh", "tools/verify_packaged_policy.sh"} {
		if err := writeFile(filepath.Join(root, path), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	for path, data := range map[string]string{"manifest": "schema=6\nrelease_id=" + plan.ReleaseID + "\n", "share/sophia-niltempus-desktop/desktop.kdl": fixtureProfile, "share/wayland-sessions/" + desktopFile: desktopEntry(), "share/wayland-sessions/" + ipcDesktopFile: ipcDesktopEntry()} {
		if err := writeFile(filepath.Join(root, path), []byte(data), 0644); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeFile(filepath.Join(root, "share/sophia-niltempus-desktop/desktop-ipc.kdl"), []byte(fixtureProfile), 0644); err != nil {
		t.Fatal(err)
	}
	if err := sealRelease(root, plan); err != nil {
		t.Fatal(err)
	}
	return root, plan
}

func TestVerifyReleaseAndCurrentLink(t *testing.T) {
	root, _ := fixtureRelease(t)
	if _, err := verifyRelease(root); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(t.TempDir(), "current")
	if err := os.Symlink(root, link); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyRelease(link); err != nil {
		t.Fatal(err)
	}
}

func TestVerifyRefusesReleaseDamage(t *testing.T) {
	cases := map[string]func(string, Plan) error{
		"modified binary": func(root string, _ Plan) error {
			return os.WriteFile(filepath.Join(root, "target/release/hagia"), []byte("wrong"), 0755)
		},
		"missing binary": func(root string, _ Plan) error { return os.Remove(filepath.Join(root, "target/release/lom")) },
		"extra file": func(root string, _ Plan) error {
			return os.WriteFile(filepath.Join(root, "unexpected"), []byte("extra"), 0644)
		},
		"lost executable": func(root string, _ Plan) error { return os.Chmod(filepath.Join(root, "target/release/hagia"), 0644) },
		"unsafe symlink":  func(root string, _ Plan) error { return os.Symlink("/etc/passwd", filepath.Join(root, "unsafe")) },
		"altered sums": func(root string, _ Plan) error {
			return os.WriteFile(filepath.Join(root, "SHA256SUMS"), []byte(""), 0644)
		},
		"source provenance": func(root string, _ Plan) error {
			var m Manifest
			if err := readJSON(filepath.Join(root, "desktop-manifest.json"), &m); err != nil {
				return err
			}
			m.Plan.ProfileSHA256 = "changed"
			return writeJSON(filepath.Join(root, "desktop-manifest.json"), m)
		},
		"unsafe manifest path": func(root string, _ Plan) error {
			var m Manifest
			if err := readJSON(filepath.Join(root, "desktop-manifest.json"), &m); err != nil {
				return err
			}
			m.Files["../outside"] = FileRecord{}
			return writeJSON(filepath.Join(root, "desktop-manifest.json"), m)
		},
		"resealed missing binary": func(root string, p Plan) error {
			if err := os.Remove(filepath.Join(root, "target/release/lom")); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed missing IPC profile": func(root string, p Plan) error {
			if err := os.Remove(filepath.Join(root, "share/sophia-niltempus-desktop/desktop-ipc.kdl")); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed wrong identity": func(root string, p Plan) error {
			if err := os.WriteFile(filepath.Join(root, "manifest"), []byte("release_id=wrong\n"), 0644); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
	}
	for name, mutate := range cases {
		t.Run(name, func(t *testing.T) {
			root, p := fixtureRelease(t)
			if err := mutate(root, p); err != nil {
				t.Fatal(err)
			}
			if _, err := verifyRelease(root); err == nil {
				t.Fatal("accepted damaged release")
			}
		})
	}
}

func TestLegacyReleaseWithoutIPCEntryRemainsAvailableForRollback(t *testing.T) {
	root, plan := fixtureRelease(t)
	plan.Schema = 1
	plan.ReleaseID = releaseID(plan)
	if err := writeFile(filepath.Join(root, "manifest"), []byte("schema=6\nrelease_id="+plan.ReleaseID+"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{ipcLauncher, "share/wayland-sessions/" + ipcDesktopFile, "share/sophia-niltempus-desktop/desktop-ipc.kdl"} {
		if err := os.Remove(filepath.Join(root, path)); err != nil {
			t.Fatal(err)
		}
	}
	if err := sealRelease(root, plan); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyRelease(root); err != nil {
		t.Fatal(err)
	}
	// A half-present IPC entry is malformed even in the older format.
	if err := installRelease(root, Locations{}); err == nil || !strings.Contains(err.Error(), "new installations require release schema 2") {
		t.Fatalf("fresh schema-1 install must stop before sudo: %v", err)
	}
	if err := writeFile(filepath.Join(root, ipcLauncher), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := sealRelease(root, plan); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyRelease(root); err == nil {
		t.Fatal("accepted incomplete IPC pair")
	}
	if err := writeFile(filepath.Join(root, "share/wayland-sessions/"+ipcDesktopFile), []byte(ipcDesktopEntry()), 0644); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(root, ipcLauncher), []byte(ipcSessionLauncher()), 0755); err != nil {
		t.Fatal(err)
	}
	if err := sealRelease(root, plan); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyRelease(root); err == nil {
		t.Fatal("accepted IPC launcher with missing named profile")
	}
}

func TestBuildEnvironmentDropsSessionAndBinaryOverrides(t *testing.T) {
	env := buildEnvironment([]string{"HOME=/home/test", "PATH=/usr/bin", "SOPHIA_HAGIA_BIN=/old/hagia", "SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1", "DISPLAY=:77", "WAYLAND_DISPLAY=socket", "CARGO_TARGET_DIR=/other/tree", "CARGO_BUILD_JOBS=100"})
	joined := strings.Join(env, "\n")
	for _, absent := range []string{"SOPHIA_", "DISPLAY=", "/other/tree", "JOBS=100"} {
		if strings.Contains(joined, absent) {
			t.Fatal(joined)
		}
	}
	for _, present := range []string{"HOME=/home/test", "CARGO_BUILD_JOBS=2", "CARGO_NET_OFFLINE=true", "XDG_RUNTIME_DIR=/tmp/runtime"} {
		if !strings.Contains(joined, present) {
			t.Fatal(joined)
		}
	}
}

func TestBuildLockReleasesAfterOwnerCloses(t *testing.T) {
	loc := Locations{Cache: t.TempDir()}
	unlock, err := buildLock(loc)
	if err != nil {
		t.Fatal(err)
	}
	if other, err := buildLock(loc); err == nil {
		other()
		unlock()
		t.Fatal("admitted concurrent build")
	}
	unlock()
	other, err := buildLock(loc)
	if err != nil {
		t.Fatal(err)
	}
	other()
}
