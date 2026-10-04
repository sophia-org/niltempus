package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	kdl "github.com/calico32/kdl-go"
)

// Upgrade only a private fixture; neither the installed desktop nor the
// operator's selected release is modified. This is launcher/install evidence,
// not a claim that this assembled test fixture is a production release.
func TestComponentInstallationInPrivateMounts(t *testing.T) {
	artifact := os.Getenv("DESKTOP_INSTALLER_TEST_RELEASE")
	binary := os.Getenv("DESKTOP_INSTALLER_TEST_BINARY")
	if artifact == "" || binary == "" {
		t.Skip("requires a built release and installer binary")
	}
	m, err := verifyRelease(artifact)
	if err != nil {
		t.Fatal(err)
	}
	fixture := t.TempDir()
	candidate := filepath.Join(fixture, "candidate")
	if out, err := exec.Command("cp", "-a", artifact, candidate).CombinedOutput(); err != nil {
		t.Fatalf("%v %s", err, out)
	}
	oldID := m.Plan.ReleaseID
	m.Plan.ComponentUpdates = true
	m.Plan.InstallerSHA256, err = fileDigest(binary)
	if err != nil {
		t.Fatal(err)
	}
	m.Plan.ReleaseID = releaseID(m.Plan)
	for _, name := range []string{"manifest", "share/sophia-niltempus-desktop/desktop.kdl"} {
		p := filepath.Join(candidate, name)
		data, err := os.ReadFile(p)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(strings.ReplaceAll(string(data), oldID, m.Plan.ReleaseID)), 0644); err != nil {
			t.Fatal(err)
		}
	}
	// This test covers the generic three-component stack. A release whose
	// sealed profile names a lock provider needs a selected kleis for
	// component-profile; that path is proved by the kleis release sequence. Here
	// the test-only copy drops the lock provider and keeps everything else.
	if err := withoutLockProvider(filepath.Join(candidate, "share/sophia-niltempus-desktop/desktop.kdl")); err != nil {
		t.Fatal(err)
	}
	if err := copyFile(binary, filepath.Join(candidate, "target/release/niltempus"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(candidate, "bin/sophia-niltempus-desktop-session"), []byte(sessionLauncher()), 0755); err != nil {
		t.Fatal(err)
	}
	if err := sealRelease(candidate, m.Plan); err != nil {
		t.Fatal(err)
	}
	if err := writeFile(filepath.Join(fixture, "fakebin/sudo"), []byte("#!/bin/sh\nexec \"$@\"\n"), 0755); err != nil {
		t.Fatal(err)
	}
	script := `set -eu
"$1" install /tmp/fixture/candidate
state="$XDG_STATE_HOME/sophia-niltempus-desktop"
for component in hagia lom bemenu; do test -f "$state/components/$component/selection.json"; done
bar="$state/components/lom/current"
menu="$state/components/bemenu/current"
wm="$state/development/hagia"
sha256sum "$bar" "$menu" "$wm" > /tmp/before
stat -c '%i:%a:%Y' "$bar" "$menu" "$wm" > /tmp/identities
"$1" install /tmp/fixture/candidate
sha256sum -c /tmp/before
stat -c '%i:%a:%Y' "$bar" "$menu" "$wm" > /tmp/after
cmp /tmp/identities /tmp/after
"$1" component-profile /opt/sophia-niltempus-desktop/current > /tmp/profile.kdl
grep -F "$bar" /tmp/profile.kdl
grep -F "$menu" /tmp/profile.kdl
grep -F "$wm" /tmp/profile.kdl
# An inactive component cannot be signalled, and the selection is unchanged.
if "$1" restart lom; then exit 90; fi
sha256sum -c /tmp/before
# Tampering cannot become a selected rollback binary or a login profile.
printf changed >> "$bar"
if "$1" component-profile /opt/sophia-niltempus-desktop/current; then exit 91; fi
`
	args := []string{"--die-with-parent", "--unshare-pid", "--unshare-net", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--tmpfs", "/run/user", "--tmpfs", "/opt", "--tmpfs", "/usr/share/wayland-sessions", "--bind", fixture, "/tmp/fixture", "--dir", "/tmp/home", "--setenv", "HOME", "/tmp/home", "--setenv", "XDG_STATE_HOME", "/tmp/home/.local/state", "--setenv", "XDG_CACHE_HOME", "/tmp/home/.cache", "--setenv", "PATH", "/tmp/fixture/fakebin:/usr/bin", "--", "bash", "-c", script, "fixture", binary}
	cmd := exec.Command("bwrap", args...)
	cmd.Env = buildEnvironment(os.Environ())
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("component install failed: %v\n%s", err, out)
	}
}

// withoutLockProvider removes a session lock-provider node, if any, leaving the
// rest of the profile as parsed.
func withoutLockProvider(path string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	doc, err := kdl.ParseString(string(data), kdl.WithVersion(kdl.Version2), kdl.WithDuplicateProperties(kdl.DupError))
	if err != nil {
		return err
	}
	session, err := uniqueNode(doc, "session")
	if err != nil {
		return err
	}
	if len(session.Children().GetNodes("lock-provider")) == 0 {
		return nil
	}
	session.Children().RemoveNodes("lock-provider")
	text, err := kdl.EmitToString(doc, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
	if err != nil {
		return err
	}
	return os.WriteFile(path, []byte(text), 0644)
}
