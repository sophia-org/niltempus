package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// Legacy (plan schema 2) launcher and entry contents, as installed releases
// carry them. New releases never generate these.
const legacyIPCLauncher = "#!/bin/bash\nexec \"$release/bin/sophia-hagia-session\" --wm-transport=current-ipc # share/sophia-niltempus-desktop/desktop-ipc.kdl\n"
const legacyIPCEntry = "[Desktop Entry]\nName=Sophia niltempus Desktop (current IPC)\nExec=" + prefix + "/current/" + ipcLauncher + "\nType=Application\n"

// legacyFixtureRelease is an installed plan-schema-2 release (external
// manifest schema 6) with its current-IPC entry.
func legacyFixtureRelease(t *testing.T) (string, Plan) {
	t.Helper()
	root := t.TempDir()
	plan := Plan{Schema: 2, ProfileSHA256: digest([]byte("profile")), InstallerSHA256: digest([]byte("installer"))}
	plan.ReleaseID = releaseID(plan)
	for _, name := range []string{"sophia", "hagia", "narthex", "lom", "bemenu-sophia"} {
		if err := writeFile(filepath.Join(root, "target/release", name), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	for _, path := range []string{"bin/sophia-niltempus-desktop-session", "bin/sophia-hagia-session", "tools/install_live_session.sh", "tools/activate_live_session_release.sh", "tools/verify_packaged_policy.sh"} {
		if err := writeFile(filepath.Join(root, path), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeFile(filepath.Join(root, ipcLauncher), []byte(legacyIPCLauncher), 0755); err != nil {
		t.Fatal(err)
	}
	for path, data := range map[string]string{"manifest": "schema=6\nrelease_id=" + plan.ReleaseID + "\n", "share/sophia-niltempus-desktop/desktop.kdl": fixtureProfile, "share/wayland-sessions/" + desktopFile: desktopEntry(), "share/wayland-sessions/" + ipcDesktopFile: legacyIPCEntry} {
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

const fixtureSDKRevision = "841563d614ed8540472f0edfa7f4cddaafe3fdde"

func fixtureSDKManifest(revision string) string {
	return `{"schema":1,"repository":"https://github.com/sophia-org/sophia-desktop-sdk-c","revision":"` + revision + `","files":{}}` + "\n"
}

// fixtureRelease is a plan-schema-3 (9P-only) release with an external
// schema-7 manifest bound to Hagia's vendored C SDK.
func fixtureRelease(t *testing.T) (string, Plan) {
	t.Helper()
	root := t.TempDir()
	plan := Plan{
		Schema:          currentPlanSchema,
		Sources:         map[string]Source{"sophia": {Commit: strings.Repeat("1", 40)}, "hagia": {Commit: strings.Repeat("2", 40)}, "narthex": {Commit: strings.Repeat("3", 40)}},
		ProfileSHA256:   digest([]byte("profile")),
		InstallerSHA256: digest([]byte("installer")),
		Niltempus:       &IntegrationPlan{Source: Source{Commit: strings.Repeat("4", 40)}, CargoHome: "/cache", CargoLockSHA256: "lock"},
		Inputs: &PlanInputs{
			HagiaNimDeps:      NimDepsInput{"/reviewed/hagia.nim-deps", strings.Repeat("a", 64)},
			NarthexNimDeps:    NimDepsInput{"/reviewed/narthex.nim-deps", strings.Repeat("b", 64)},
			HagiaCSDKRevision: fixtureSDKRevision,
		},
	}
	plan.ReleaseID = releaseID(plan)
	for _, path := range []string{"target/release/sophia", "target/release/hagia", "target/release/narthex", "target/release/lom", "target/release/bemenu-sophia", "target/release/sophia-integration-xtask", "target/release/active-session-preflight", "bin/sophia-hagia-session", "bin/sophia-session", "tools/install_live_session.sh", "tools/activate_live_session_release.sh", "tools/verify_packaged_policy.sh", "tools/session/run_desktop_session.sh", "tools/lib/live_session_surface.sh"} {
		if err := writeFile(filepath.Join(root, path), []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	profile, err := renderProfile(fixtureProfile, filepath.Join(prefix, "releases", plan.ReleaseID, "target/release"))
	if err != nil {
		t.Fatal(err)
	}
	for path, data := range map[string]string{
		"bin/sophia-niltempus-desktop-session":       sessionLauncher(),
		"tools/lib/activation_ledger.sh":             "# ledger\n",
		"share/sophia-niltempus-desktop/desktop.kdl": profile,
		"share/wayland-sessions/" + desktopFile:      desktopEntry(),
		"share/sophia-policy/hagia/default.kdl":      "profile",
		sealedCSDKManifest:                           fixtureSDKManifest(fixtureSDKRevision),
	} {
		mode := os.FileMode(0644)
		if strings.HasPrefix(path, "bin/") {
			mode = 0755
		}
		if err := writeFile(filepath.Join(root, path), []byte(data), mode); err != nil {
			t.Fatal(err)
		}
	}
	writeSchema7Metadata(t, root, plan, nil)
	return root, plan
}

// writeSchema7Metadata writes the external manifest the packager would, then
// reseals. mutate may alter the text first.
func writeSchema7Metadata(t *testing.T, root string, plan Plan, mutate func(string) string) {
	t.Helper()
	hash := func(path string) string {
		h, err := fileDigest(filepath.Join(root, path))
		if err != nil {
			t.Fatal(err)
		}
		return h
	}
	metadata := "schema=7\nversion=0.1.0\ncommit=" + plan.Sources["sophia"].Commit + "\nrelease_id=" + plan.ReleaseID +
		"\nbuilt_at_utc=2026-09-27T00:00:00Z\nhagia_included=true\nhagia_source_commit=" + plan.Sources["hagia"].Commit +
		"\nhagia_default_profile_sha256=" + hash("share/sophia-policy/hagia/default.kdl") +
		"\nhagia_binary_sha256=" + hash("target/release/hagia") + "\nhagia_shell_binary_sha256=" + hash("target/release/narthex") +
		"\nnarthex_source_commit=" + plan.Sources["narthex"].Commit + "\nintegration_commit=" + plan.Niltempus.Source.Commit +
		"\nhagia_c_sdk_revision=" + plan.Inputs.HagiaCSDKRevision + "\nhagia_c_sdk_manifest_sha256=" + hash(sealedCSDKManifest) + "\n"
	if mutate != nil {
		metadata = mutate(metadata)
	}
	if err := writeFile(filepath.Join(root, "manifest"), []byte(metadata), 0644); err != nil {
		t.Fatal(err)
	}
	if err := sealRelease(root, plan); err != nil {
		t.Fatal(err)
	}
}

func TestVerifyReleaseAndCurrentLink(t *testing.T) {
	root, _ := fixtureRelease(t)
	if _, err := verifyRelease(root); err != nil {
		t.Fatal(err)
	}
	legacy, _ := legacyFixtureRelease(t)
	if _, err := verifyRelease(legacy); err != nil {
		t.Fatalf("installed legacy release: %v", err)
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
		"private profile directory": func(root string, _ Plan) error {
			return os.Chmod(filepath.Join(root, "share/sophia-niltempus-desktop"), 0700)
		},
		"private release directory":  func(root string, _ Plan) error { return os.Chmod(root, 0700) },
		"writable release directory": func(root string, _ Plan) error { return os.Chmod(root, 0775) },
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
		"resealed IPC profile": func(root string, p Plan) error {
			if err := writeFile(filepath.Join(root, "share/sophia-niltempus-desktop/desktop-ipc.kdl"), []byte(fixtureProfile), 0644); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed IPC login entry": func(root string, p Plan) error {
			if err := writeFile(filepath.Join(root, "share/wayland-sessions/"+ipcDesktopFile), []byte(legacyIPCEntry), 0644); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed IPC bar": func(root string, p Plan) error {
			path := filepath.Join(root, "share/sophia-niltempus-desktop/desktop.kdl")
			data, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			changed := strings.Replace(string(data), `transport "9p2000.L"`, `transport "current-ipc"`, 1)
			if changed == string(data) {
				return os.ErrInvalid
			}
			if err := os.WriteFile(path, []byte(changed), 0644); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed IPC launcher": func(root string, p Plan) error {
			if err := writeFile(filepath.Join(root, "bin/sophia-niltempus-desktop-session"), []byte(strings.ReplaceAll(sessionLauncher(), "9p2000.L", "current-ipc")), 0755); err != nil {
				return err
			}
			return sealRelease(root, p)
		},
		"resealed missing ledger library": func(root string, p Plan) error {
			if err := os.Remove(filepath.Join(root, "tools/lib/activation_ledger.sh")); err != nil {
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

func TestReleaseDirectoriesUnderPrivateUmask(t *testing.T) {
	if os.Getenv("NILTEMPUS_PRIVATE_UMASK_TEST") != "1" {
		cmd := exec.Command("sh", "-c", `umask 077; exec "$@"`, "umask-test", os.Args[0], "-test.run=^TestReleaseDirectoriesUnderPrivateUmask$")
		cmd.Env = append(os.Environ(), "NILTEMPUS_PRIVATE_UMASK_TEST=1")
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("private-umask child: %v: %s", err, out)
		}
		return
	}
	root, _ := fixtureRelease(t)
	if _, err := verifyRelease(root); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{root, filepath.Join(root, "share/sophia-niltempus-desktop")} {
		info, err := os.Stat(path)
		if err != nil || info.Mode().Perm() != 0755 {
			t.Fatalf("published directory: %v: %v", info, err)
		}
	}
	info, err := os.Stat(filepath.Dir(root))
	if err != nil || info.Mode().Perm() != 0700 {
		t.Fatalf("private parent changed: %v: %v", info, err)
	}
	// This remains readable to its owner, but would fail after root's cp -a.
	if err := os.Chmod(filepath.Join(root, "share/sophia-niltempus-desktop"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := installRelease(root, Locations{}); err == nil || !strings.Contains(err.Error(), "release directory must have mode 0755") {
		t.Fatalf("install must reject private directories before sudo: %v", err)
	}
}

func TestLegacyReleaseWithoutIPCEntryRemainsAvailableForRollback(t *testing.T) {
	root, plan := legacyFixtureRelease(t)
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
	if err := installRelease(root, Locations{}); err == nil || !strings.Contains(err.Error(), "new installations require plan schema 3") {
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
	if err := writeFile(filepath.Join(root, "share/wayland-sessions/"+ipcDesktopFile), []byte(legacyIPCEntry), 0644); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(root, ipcLauncher), []byte(legacyIPCLauncher), 0755); err != nil {
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
