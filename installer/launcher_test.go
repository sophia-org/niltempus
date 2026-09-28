package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// runLauncher runs a launcher against a stubbed Sophia wrapper that only prints
// its environment and argv; it never runs a session.
func runLauncher(t *testing.T, name, script string) (root, wm, out string) {
	t.Helper()
	root = t.TempDir()
	launcher := filepath.Join(root, name)
	if err := writeFile(launcher, []byte(script), 0755); err != nil {
		t.Fatal(err)
	}
	stub := `#!/bin/sh
printf '%s\n' "$SOPHIA_HAGIA_BIN" "$SOPHIA_DESKTOP_PROFILE" "${SOPHIA_HAGIA_PROFILE_MODE-unset}" "${SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE-unset}" "$SOPHIA_INSTALL_PREFIX" "$@"
`
	if err := writeFile(filepath.Join(root, "bin/sophia-hagia-session"), []byte(stub), 0755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(launcher, "one argument", "second")
	if err := writeFile(filepath.Join(root, "target/release/niltempus"), []byte("#!/bin/sh\ntest \"$1\" = component-profile || exit 99\nprintf '# profile fixture\\n'\n"), 0755); err != nil {
		t.Fatal(err)
	}
	state := filepath.Join(root, "user state")
	wm = filepath.Join(state, "sophia-niltempus-desktop/development/hagia")
	wmSource := "#!/bin/sh\ntest \"$#\" = 2 && test \"$1\" = config && test \"$2\" = check-environment-contract || exit 99\nprintf '%s\\n' '" + policyEnvironmentContract + "'\n"
	if err := writeFile(wm, []byte(wmSource), 0755); err != nil {
		t.Fatal(err)
	}
	cmd.Env = append(os.Environ(), "XDG_RUNTIME_DIR="+root, "XDG_STATE_HOME="+state, "SOPHIA_HAGIA_BIN=/stale/hagia", "SOPHIA_DESKTOP_PROFILE=/stale/profile", "SOPHIA_HAGIA_PROFILE_MODE=packaged-promotion", "SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1")
	got, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("%v: %s", err, got)
	}
	return root, wm, string(got)
}

func TestDefaultLauncherRunsTheWMOverNineP(t *testing.T) {
	root, wm, out := runLauncher(t, "bin/sophia-niltempus-desktop-session", sessionLauncher())
	profile := strings.Split(out, "\n")[1]
	if !strings.HasPrefix(profile, filepath.Join(root, "niltempus-components.")) {
		t.Fatal(profile)
	}
	if data, err := os.ReadFile(profile); err != nil || string(data) != "# profile fixture\n" {
		t.Fatal("missing private profile", err)
	}
	want := strings.Join([]string{wm, profile, "unset", "unset", prefix, "--wm-process=" + wm, "--wm-transport=9p2000.L", "one argument", "second", ""}, "\n")
	if out != want {
		t.Fatalf("got %s; want %s", out, want)
	}
}

func TestLauncherRefusesOldOrMismatchedPersonalWM(t *testing.T) {
	for _, response := range []string{"exit 1", "exit 0", "echo incompatible"} {
		root := t.TempDir()
		entry := filepath.Join(root, "bin/sophia-niltempus-desktop-session")
		if err := writeFile(entry, []byte(sessionLauncher()), 0755); err != nil {
			t.Fatal(err)
		}
		state := filepath.Join(root, "state")
		wm := filepath.Join(state, "sophia-niltempus-desktop/development/hagia")
		if err := writeFile(wm, []byte("#!/bin/sh\n"+response+"\n"), 0755); err != nil {
			t.Fatal(err)
		}
		marker := filepath.Join(root, "started")
		if err := writeFile(filepath.Join(root, "bin/sophia-hagia-session"), []byte("#!/bin/sh\ntouch \"$STARTED\"\n"), 0755); err != nil {
			t.Fatal(err)
		}
		cmd := exec.Command(entry)
		cmd.Env = append(os.Environ(), "XDG_STATE_HOME="+state, "STARTED="+marker)
		if out, err := cmd.CombinedOutput(); err == nil {
			t.Fatalf("accepted unsupported WM: %q: %s", response, out)
		}
		if _, err := os.Stat(marker); !os.IsNotExist(err) {
			t.Fatalf("session reached before WM compatibility check: %v", err)
		}
	}
}

func TestIsolationActuallyHidesDevicesAndRuntimeSockets(t *testing.T) {
	if _, err := exec.LookPath("bwrap"); err != nil {
		t.Skip("Bubblewrap not installed")
	}
	cmd := isolated(t.TempDir(), nil, "sh", "-c", `test ! -e /dev/dri && test -z "${DISPLAY-}${WAYLAND_DISPLAY-}${SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE-}" && test "$(find /run/user -mindepth 1 -print -quit)" = ""`)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("isolation failed: %v: %s", err, out)
	}
}

func TestEntriesNameTheirLaunchers(t *testing.T) {
	if !strings.Contains(desktopEntry(), "Exec="+prefix+"/current/bin/sophia-niltempus-desktop-session\n") {
		t.Fatalf("default entry: %s", desktopEntry())
	}
	// 9P-only: the one entry never offers a current-IPC variant.
	if strings.Contains(desktopEntry(), "IPC") || !strings.Contains(desktopEntry(), "9P2000.L") {
		t.Fatalf("default entry: %s", desktopEntry())
	}
	if strings.Contains(sessionLauncher(), "current-ipc") {
		t.Fatal("the session launcher names the current-IPC wire")
	}
}
