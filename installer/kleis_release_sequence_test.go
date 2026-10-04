package main

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	kdl "github.com/calico32/kdl-go"
)

// The operator's upgrade to the kleis lock-provider release, rehearsed with
// the real CLI in private mounts. Nothing outside the fixture is written: /opt
// is private, the operator's component state and configuration are copies,
// and every source repository is visible read-only.
//
// The sequence: install the old desktop over a copy of the operator's
// selected components; an SDK-changed component is refused against it; the
// candidate installs without touching personal components; its sealed profile
// names a lock provider, so component-profile refuses while kleis is
// unselected; a kleis whose sources do not match the reviewed inputs is
// refused without changing state; hagia, bemenu and kleis are prepared from
// the reviewed configuration and selected exactly; repeating that changes
// nothing; finally the old desktop is restored by rollback plus the operator's
// backup of component state, and renders the same profile as before.
//
// Inputs (all required; the test is skipped without them):
//
//	DESKTOP_INSTALLER_TEST_BINARY           installer under test
//	DESKTOP_INSTALLER_TEST_RELEASE          candidate release artifact
//	DESKTOP_INSTALLER_TEST_OLD_RELEASE      the release being upgraded from
//	DESKTOP_INSTALLER_TEST_CONFIG           reviewed config.json for the candidate
//	DESKTOP_INSTALLER_TEST_COMPONENT_STATE  copy of the operator's state holding
//	                                        components/ and development/
//	DESKTOP_INSTALLER_TEST_STALE_KLEIS      a signed kleis commit the reviewed
//	                                        inputs must refuse
//	DESKTOP_INSTALLER_TEST_GNUPG_PUBLIC     directory holding the public
//	                                        pubring.kbx and trustdb.gpg; only
//	                                        those two files are copied, never
//	                                        private keys or agent sockets
//	DESKTOP_INSTALLER_TEST_EVIDENCE         new absolute directory that keeps
//	                                        the log, snapshots and profiles
func TestKleisReleaseSequenceInPrivateMounts(t *testing.T) {
	inputs := map[string]string{}
	for _, name := range []string{"BINARY", "RELEASE", "OLD_RELEASE", "CONFIG", "COMPONENT_STATE", "STALE_KLEIS", "GNUPG_PUBLIC", "EVIDENCE"} {
		value := os.Getenv("DESKTOP_INSTALLER_TEST_" + name)
		if value == "" {
			t.Skip("requires the candidate, old release, reviewed config, component state, stale kleis, public keyring and evidence inputs")
		}
		inputs[name] = value
	}
	candidate, err := verifyRelease(inputs["RELEASE"])
	if err != nil {
		t.Fatal(err)
	}
	old, err := verifyRelease(inputs["OLD_RELEASE"])
	if err != nil {
		t.Fatal(err)
	}
	var cfg Config
	if err := readJSON(inputs["CONFIG"], &cfg); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"hagia", "bemenu", "kleis"} {
		if _, ok := cfg.Repositories[name]; !ok {
			t.Fatalf("reviewed config has no %s repository", name)
		}
	}
	oldSelections := map[string]ComponentSelection{}
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		var s ComponentSelection
		if err := readJSON(filepath.Join(inputs["COMPONENT_STATE"], "components", name, "selection.json"), &s); err != nil {
			t.Fatal(err)
		}
		oldSelections[name] = s
	}
	if _, err := os.Lstat(filepath.Join(inputs["COMPONENT_STATE"], "components", "kleis")); !os.IsNotExist(err) {
		t.Fatal("the upgrade starts from a state without kleis")
	}

	evidence := inputs["EVIDENCE"]
	if !filepath.IsAbs(evidence) {
		t.Fatal("DESKTOP_INSTALLER_TEST_EVIDENCE must be absolute")
	}
	if err := os.Mkdir(evidence, 0700); err != nil {
		t.Fatalf("evidence directory must be new: %v", err)
	}
	fixture := t.TempDir()
	// Signed-source checks run with a replaced HOME, so they get the public
	// keyring and trust database only.
	gnupg := filepath.Join(fixture, "gnupg")
	if err := os.Mkdir(gnupg, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"pubring.kbx", "trustdb.gpg"} {
		if err := copyFile(filepath.Join(inputs["GNUPG_PUBLIC"], name), filepath.Join(gnupg, name), 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeFile(filepath.Join(fixture, "fakebin/sudo"), []byte("#!/bin/sh\nexec \"$@\"\n"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := writeJSON(filepath.Join(fixture, "config.json"), cfg); err != nil {
		t.Fatal(err)
	}
	stale := cfg
	stale.Repositories = map[string]Repository{}
	for name, repo := range cfg.Repositories {
		stale.Repositories[name] = repo
	}
	stale.Repositories["kleis"] = Repository{cfg.Repositories["kleis"].Path, inputs["STALE_KLEIS"]}
	if err := writeJSON(filepath.Join(fixture, "config.stale-kleis.json"), stale); err != nil {
		t.Fatal(err)
	}

	script := `set -eu
step() { printf '== step %s\n' "$*"; }
desktop=/opt/sophia-niltempus-desktop
state="$XDG_STATE_HOME/sophia-niltempus-desktop"
conf="$HOME/.config/sophia-niltempus-desktop"
out=/tmp/fixture/out
mkdir -p "$out" "$conf" "$state"
cp -a /tmp/component-state/components /tmp/component-state/development "$state/"
# Component state only: selections, versions, current executables and the
# personal Hagia with its record. Symlinks are listed by target.
snapshot() {
	(cd "$state" && { find components development -type f -print0 | sort -z | xargs -0 sha256sum
		find components development -type l -printf '%p -> %l\n' | sort
		find components development -printf '%p %y %m\n' | sort; }) > "$out/$1.state"
}
refused() { # name, expected message, command...
	name=$1 message=$2; shift 2
	if "$@" > "$out/$name.out" 2> "$out/$name.err"; then echo "accepted: $name"; exit 1; fi
	grep -qF -- "$message" "$out/$name.err" || { cat "$out/$name.err"; exit 1; }
}

step 1 install the old desktop over the operator components
"$1" install /tmp/old-release
test "$(readlink "$desktop/current")" = "releases/$2"
snapshot old
"$1" component-profile "$desktop/current" > "$out/old.profile"
cp -a "$state/components" "$state/development" /tmp/fixture/backup/

step 2 an SDK-changed component is refused against the old desktop
cp /tmp/fixture/config.json "$conf/config.json"
refused bemenu-on-old "C SDK differs from the installed desktop" "$1" prepare-component bemenu
snapshot refused-on-old
cmp "$out/old.state" "$out/refused-on-old.state"

step 3 install the candidate without touching personal components
"$1" prepare /tmp/candidate
"$1" install
test "$(readlink "$desktop/current")" = "releases/$3"
test "$(readlink "$desktop/previous")" = "releases/$2"
snapshot installed
cmp "$out/old.state" "$out/installed.state"

step 4 the sealed lock provider needs a selected kleis
refused profile-without-kleis "kleis:" "$1" component-profile "$desktop/current"

step 5 kleis outside the reviewed inputs is refused without changes
cp /tmp/fixture/config.stale-kleis.json "$conf/config.json"
refused stale-kleis "dependency manifest was not reviewed for kleis at $4" "$1" prepare-component kleis
cp /tmp/fixture/config.json "$conf/config.json"
snapshot refused-stale
cmp "$out/old.state" "$out/refused-stale.state"
test ! -e "$state/components/kleis/selection.json"

step 6 prepare hagia, bemenu and kleis from the reviewed configuration
for component in hagia bemenu kleis; do "$1" prepare-component "$component"; done
snapshot prepared
"$1" component-profile "$desktop/current" > "$out/new.profile"
cp -a "$state/components" "$state/development" /tmp/fixture/prepared/

step 7 repeating the preparation changes nothing
for component in hagia bemenu kleis; do "$1" prepare-component "$component"; done
snapshot prepared-again
cmp "$out/prepared.state" "$out/prepared-again.state"
"$1" component-profile "$desktop/current" > "$out/new-again.profile"
cmp "$out/new.profile" "$out/new-again.profile"

step 8 rollback and the operator backup restore the old desktop
"$1" rollback
test "$(readlink "$desktop/current")" = "releases/$2"
rm -rf "$state/components" "$state/development"
cp -a /tmp/fixture/backup/components /tmp/fixture/backup/development "$state/"
snapshot restored
cmp "$out/old.state" "$out/restored.state"
"$1" component-profile "$desktop/current" > "$out/restored.profile"
cmp "$out/old.profile" "$out/restored.profile"
step done
`
	for _, dir := range []string{"out", "backup", "prepared", "home", "cache"} {
		if err := os.MkdirAll(filepath.Join(fixture, dir), 0700); err != nil {
			t.Fatal(err)
		}
	}
	args := []string{"--die-with-parent", "--unshare-pid", "--unshare-net",
		"--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
		"--tmpfs", "/tmp", "--tmpfs", "/run/user", "--tmpfs", "/opt", "--tmpfs", "/usr/share/wayland-sessions",
		"--bind", fixture, "/tmp/fixture",
		"--ro-bind", inputs["OLD_RELEASE"], "/tmp/old-release",
		"--ro-bind", inputs["RELEASE"], "/tmp/candidate",
		"--ro-bind", inputs["COMPONENT_STATE"], "/tmp/component-state",
		"--chdir", "/tmp/fixture",
		"--setenv", "HOME", "/tmp/fixture/home",
		"--setenv", "XDG_STATE_HOME", "/tmp/fixture/home/.local/state",
		"--setenv", "XDG_CACHE_HOME", "/tmp/fixture/cache",
		"--setenv", "GNUPGHOME", "/tmp/fixture/gnupg",
		"--setenv", "PATH", "/tmp/fixture/fakebin:/usr/bin",
		"--", "bash", "-c", script, "fixture", inputs["BINARY"], old.Plan.ReleaseID, candidate.Plan.ReleaseID, inputs["STALE_KLEIS"]}
	// The whole sequence, including three product builds, is bounded; the
	// context kills bwrap, and --die-with-parent takes everything under it.
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, "bwrap", args...)
	cmd.Env = buildEnvironment(os.Environ())
	cmd.WaitDelay = 10 * time.Second
	out, err := cmd.CombinedOutput()
	// Keep the evidence whatever happened; t.TempDir is removed afterwards.
	if werr := writeFile(filepath.Join(evidence, "sequence.log"), out, 0600); werr != nil {
		t.Error(werr)
	}
	if cerr := exec.Command("cp", "-a", filepath.Join(fixture, "out"), filepath.Join(evidence, "out")).Run(); cerr != nil {
		t.Error(cerr)
	}
	if ctx.Err() != nil {
		t.Fatalf("release sequence exceeded its bound: %v", ctx.Err())
	}
	if err != nil {
		t.Fatalf("release sequence failed: %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "== step done") {
		t.Fatalf("release sequence stopped early:\n%s", out)
	}

	// Exact selections: hagia, bemenu and kleis follow the reviewed sources;
	// lom is untouched; each replaced selection keeps the operator's version
	// as its rollback, and kleis starts without one.
	state := filepath.Join(fixture, "prepared")
	inner := "/tmp/fixture/home/.local/state/sophia-niltempus-desktop"
	paths := map[string]string{}
	for _, name := range []string{"hagia", "lom", "bemenu", "kleis"} {
		var s ComponentSelection
		if err := readJSON(filepath.Join(state, "components", name, "selection.json"), &s); err != nil {
			t.Fatal(err)
		}
		binary := filepath.Join(state, "components", name, "current")
		paths[name] = filepath.Join(inner, "components", name, "current")
		if name == "hagia" {
			binary = filepath.Join(state, "development", "hagia")
			paths[name] = filepath.Join(inner, "development", "hagia")
		}
		hash, err := fileDigest(binary)
		if err != nil {
			t.Fatal(err)
		}
		if s.Current.SHA256 != hash || s.Current.Name != name {
			t.Fatalf("%s selection does not name its current executable", name)
		}
		if name == "lom" {
			if !reflect.DeepEqual(s, oldSelections[name]) {
				t.Fatalf("lom selection changed: %+v", s)
			}
			continue
		}
		want := cfg.Repositories[name]
		if s.Current.Source.Commit != want.Reference || s.Current.Source.Path != want.Path || s.Current.Source.Signature != "G" {
			t.Fatalf("%s selected %+v, not the reviewed %+v", name, s.Current.Source, want)
		}
		switch name {
		case "kleis":
			if s.Previous != nil {
				t.Fatalf("first kleis selection has a previous version: %+v", s.Previous)
			}
		default:
			if s.Previous == nil || *s.Previous != oldSelections[name].Current {
				t.Fatalf("%s rollback is not the operator's selection: %+v", name, s.Previous)
			}
		}
	}
	var personal DevelopmentHagia
	if err := readJSON(filepath.Join(state, "development", "hagia.json"), &personal); err != nil {
		t.Fatal(err)
	}
	if personal.Source.Commit != cfg.Repositories["hagia"].Reference {
		t.Fatalf("personal Hagia record names %s", personal.Source.Commit)
	}

	// The login profile is the sealed candidate profile with exactly four
	// executables substituted. Everything else, including the lock provider's
	// config and gpu and every shell component setting, is unchanged.
	sealed, err := os.ReadFile(filepath.Join(inputs["RELEASE"], "share/sophia-niltempus-desktop/desktop.kdl"))
	if err != nil {
		t.Fatal(err)
	}
	rendered, err := os.ReadFile(filepath.Join(fixture, "out", "new.profile"))
	if err != nil {
		t.Fatal(err)
	}
	assertProfileSubstitution(t, string(sealed), string(rendered), paths)
}

// assertProfileSubstitution requires rendered to equal sealed after replacing
// the WM and the bar, launcher and lock-provider executables with paths, and
// requires each replacement to have happened.
func assertProfileSubstitution(t *testing.T, sealed, rendered string, paths map[string]string) {
	t.Helper()
	if err := checkProfileSubstitution(sealed, rendered, paths); err != nil {
		t.Fatal(err)
	}
}

type profileMismatch struct{ error }

func checkProfileSubstitution(sealed, rendered string, paths map[string]string) (err error) {
	defer func() {
		if r := recover(); r != nil {
			mismatch, ok := r.(profileMismatch)
			if !ok {
				panic(r)
			}
			err = mismatch.error
		}
	}()
	fail := func(format string, args ...any) { panic(profileMismatch{fmt.Errorf(format, args...)}) }
	parse := func(source string) *kdl.Document {
		doc, err := kdl.ParseString(source, kdl.WithVersion(kdl.Version2), kdl.WithDuplicateProperties(kdl.DupError))
		if err != nil {
			fail("%v", err)
		}
		return doc
	}
	session := func(doc *kdl.Document) *kdl.Node {
		nodes := doc.GetNodes("session")
		if len(nodes) != 1 {
			fail("expected one session, found %d", len(nodes))
		}
		return nodes[0]
	}
	executable := func(parent *kdl.Node, child string) *kdl.Node {
		nodes := parent.Children().GetNodes(child)
		if len(nodes) != 1 || len(nodes[0].Arguments()) != 1 {
			fail("expected one %s with one argument", child)
		}
		return nodes[0]
	}
	setting := func(parent *kdl.Node, child string) string {
		nodes := parent.Children().GetNodes(child)
		if len(nodes) != 1 || len(nodes[0].Arguments()) == 0 {
			fail("expected one %s setting", child)
		}
		return nodes[0].Arg(0).String()
	}
	want := parse(sealed)
	got := parse(rendered)
	wantSession, gotSession := session(want), session(got)

	// Positive: each substituted executable names the selected component.
	substitute := func(wantNode, gotNode *kdl.Node, path string) {
		if gotNode.Arg(0).String() != path {
			fail("%s is %q, not %q", gotNode.Name(), gotNode.Arg(0).String(), path)
		}
		if wantNode.Arg(0).String() == path {
			fail("sealed profile already names %q", path)
		}
		wantNode.SetArg(0, kdl.NewString(path))
	}
	substitute(executable(wantSession, "window-manager"), executable(gotSession, "window-manager"), paths["hagia"])
	shells := map[string]*kdl.Node{}
	for _, node := range gotSession.Children().GetNodes("shell-component") {
		shells[node.Arg(1).String()] = node
	}
	for _, node := range wantSession.Children().GetNodes("shell-component") {
		role := node.Arg(1).String()
		name := map[string]string{"bar": "lom", "application-launcher": "bemenu"}[role]
		if name == "" || shells[role] == nil {
			fail("shell component %q missing or unknown", role)
		}
		substitute(executable(node, "executable"), executable(shells[role], "executable"), paths[name])
	}
	wantLock := wantSession.Children().GetNodes("lock-provider")
	gotLock := gotSession.Children().GetNodes("lock-provider")
	if len(wantLock) != 1 || len(gotLock) != 1 {
		fail("expected one lock provider in sealed and rendered profiles, found %d and %d", len(wantLock), len(gotLock))
	}
	substitute(executable(wantLock[0], "executable"), executable(gotLock[0], "executable"), paths["kleis"])
	// The lock provider keeps its own settings, and they are the sealed ones.
	for _, child := range []string{"config", "gpu"} {
		if setting(gotLock[0], child) != setting(wantLock[0], child) {
			fail("lock provider %s changed", child)
		}
	}

	// Negative: with those four values substituted, nothing else differs.
	emit := func(doc *kdl.Document) string {
		text, err := kdl.EmitToString(doc, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
		if err != nil {
			fail("%v", err)
		}
		return text
	}
	if a, b := emit(want), emit(got); a != b {
		fail("rendered profile differs beyond the four executables:\n--- want\n%s\n--- got\n%s", a, b)
	}
	// No path from the build or an earlier release survives.
	for _, stale := range []string{"desktop-releases/", "/target/release/"} {
		if strings.Contains(emit(got), stale) {
			fail("rendered profile still names %q", stale)
		}
	}
	return nil
}
