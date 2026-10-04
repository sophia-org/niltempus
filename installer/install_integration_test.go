package main

import (
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// This exercises the real CLI and selected Sophia install/activation scripts.
// /opt and display-manager entries exist only in private mounts; sudo is a
// fixture that executes without privilege. No live installation is possible.
func TestInstallAndRollbackInPrivateMounts(t *testing.T) {
	artifact := os.Getenv("DESKTOP_INSTALLER_TEST_RELEASE")
	binary := os.Getenv("DESKTOP_INSTALLER_TEST_BINARY")
	if artifact == "" || binary == "" {
		t.Skip("requires a built release and installer binary")
	}
	manifest, err := verifyRelease(artifact)
	if err != nil {
		t.Fatal(err)
	}
	fixture := t.TempDir()
	second := filepath.Join(fixture, "second")
	if out, err := exec.Command("cp", "-a", artifact, second).CombinedOutput(); err != nil {
		t.Fatalf("%v: %s", err, out)
	}
	plan := manifest.Plan
	// A second spelling of the same Git commit gives the fixture a distinct
	// release identity without falsifying the packaged installer's digest.
	plan.Sources = maps.Clone(plan.Sources)
	source := plan.Sources["lom"]
	source.Reference = source.Commit + "^{commit}"
	plan.Sources["lom"] = source
	plan.ReleaseID = releaseID(plan)
	for _, path := range []string{"manifest", "share/sophia-niltempus-desktop/desktop.kdl"} {
		file := filepath.Join(second, path)
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(file, []byte(strings.ReplaceAll(string(data), manifest.Plan.ReleaseID, plan.ReleaseID)), 0644); err != nil {
			t.Fatal(err)
		}
	}
	if err := sealRelease(second, plan); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyRelease(second); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(fixture, "fakebin/sudo"), []byte("#!/bin/sh\nexec \"$@\"\n"), 0755); err != nil {
		t.Fatal(err)
	}
	script := `set -eu
test ! -e /dev/dri
test ! -e /opt/sophia
"$1" prepare "$2"
"$1" install "$2"
test "$(readlink /opt/sophia-niltempus-desktop/current)" = "releases/$3"
test -z "$(find /opt/sophia-niltempus-desktop/releases/"$3" -type d ! -perm 0755 -print)"
wm="$XDG_STATE_HOME/sophia-niltempus-desktop/development/hagia"
test -x "$wm" && test -O "$wm"
cmp "$wm" "$2/target/release/hagia"
printf '\npersonal-wm-update\n' >> "$wm"
chmod 700 "$wm"
touch -t 200001010000 "$wm"
# A managed update has matching binary and selection records. Historical
# releases have no selection record and retain their original fixture behavior.
python3 - "$wm" "$XDG_STATE_HOME/sophia-niltempus-desktop" <<'PY'
import hashlib, json, pathlib, shutil, sys
wm, state = map(pathlib.Path, sys.argv[1:])
record = state / 'components/hagia/selection.json'
if record.exists():
    sha = hashlib.sha256(wm.read_bytes()).hexdigest()
    selection = json.loads(record.read_text())
    selection['previous'] = selection['current'].copy()
    selection['current']['sha256'] = sha
    record.write_text(json.dumps(selection))
    version = record.parent / 'versions' / sha
    shutil.copyfile(wm, version)
    version.chmod(0o555)
    metadata = wm.parent / 'hagia.json'
    value = json.loads(metadata.read_text())
    value['sha256'] = sha
    metadata.write_text(json.dumps(value))
PY
cp -p "$wm" /tmp/fixture/personal-wm
cp -p "$(dirname "$wm")/hagia.json" /tmp/fixture/personal-wm.json
wm_identity=$(stat -c '%i:%a:%Y' "$wm")
grep -q 'control host-admin' /opt/sophia-niltempus-desktop/current/share/sophia-niltempus-desktop/desktop.kdl
"$1" install "$2"
"$1" install /tmp/fixture/second
test "$(readlink /opt/sophia-niltempus-desktop/current)" = "releases/$4"
test "$(readlink /opt/sophia-niltempus-desktop/previous)" = "releases/$3"
"$1" install /tmp/fixture/second
test "$(readlink /opt/sophia-niltempus-desktop/previous)" = "releases/$3"
"$1" rollback
test "$(readlink /opt/sophia-niltempus-desktop/current)" = "releases/$3"
cmp "$wm" /tmp/fixture/personal-wm
cmp "$(dirname "$wm")/hagia.json" /tmp/fixture/personal-wm.json
test "$(stat -c '%i:%a:%Y' "$wm")" = "$wm_identity"
"$1" status
test -f /usr/share/wayland-sessions/sophia-niltempus-desktop.desktop
test ! -e /usr/share/wayland-sessions/sophia-niltempus-desktop-ipc.desktop
test ! -e /opt/sophia-niltempus-desktop/current/share/sophia-niltempus-desktop/desktop-ipc.kdl
test "$(find /usr/share/wayland-sessions -type f | wc -l)" = 1
awk 'NF == 3' /opt/sophia-niltempus-desktop/activated-releases | grep -q "^$3 "
awk 'NF == 3' /opt/sophia-niltempus-desktop/activated-releases | grep -q "^$4 "
test ! -e /opt/sophia
`
	args := []string{"--die-with-parent", "--unshare-pid", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--tmpfs", "/run/user", "--tmpfs", "/opt", "--tmpfs", "/usr/share/wayland-sessions", "--bind", fixture, "/tmp/fixture", "--dir", "/tmp/home", "--chdir", "/tmp/fixture", "--setenv", "HOME", "/tmp/home", "--setenv", "XDG_STATE_HOME", "/tmp/home/.local/state", "--setenv", "XDG_CACHE_HOME", "/tmp/home/.cache", "--setenv", "PATH", "/tmp/fixture/fakebin:/usr/bin", "--", "bash", "-c", script, "fixture", binary, artifact, manifest.Plan.ReleaseID, plan.ReleaseID}
	cmd := exec.Command("bwrap", args...)
	cmd.Env = buildEnvironment(os.Environ())
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("private install/rollback failed: %v\n%s", err, out)
	}
	t.Log("real CLI: install, repeat install, second install, rollback and status passed in private mounts")
}
