package main

import (
	kdl "github.com/calico32/kdl-go"
	"path/filepath"
	"strings"
	"testing"
)

const fixtureProfile = `schema 1
policy { layout scroller; arrow-crosses-outputs #false; }
shortcut { bind "Super+Return" "session:spawn-terminal"; }
session {
    window-manager "/old/hagia"
    shell-component "panel" "bar" { executable "/old/lom"; config "/home/test/lom.kdl"; gpu direct; }
    shell-component "menu" "application-launcher" { executable "/old/bemenu"; gpu denied; }
    application "terminal" { exec "/usr/bin/kitty"; }
    terminal "terminal"
    startup
}
input { keyboard { repeat-rate 40; } }
`

func TestDevelopmentProfileOverridesOnlyWM(t *testing.T) {
	result, err := renderDevelopmentProfile(fixtureProfile, "/development/hagia")
	if err != nil {
		t.Fatal(err)
	}
	want, err := kdl.ParseString(strings.Replace(fixtureProfile, "/old/hagia", "/development/hagia", 1))
	if err != nil {
		t.Fatal(err)
	}
	canonical, err := kdl.EmitToString(want, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
	if err != nil || result != canonical {
		t.Fatalf("development override changed other profile values: %v\n%s", err, result)
	}
}

func TestProfilePreservesPolicyAndPinsCompleteStack(t *testing.T) {
	binaries := `/release/a path "with quotes"/target/release`
	result, err := renderProfile(fixtureProfile, binaries)
	if err != nil {
		t.Fatal(err)
	}
	doc, err := kdl.ParseString(result)
	if err != nil {
		t.Fatal(err)
	}
	old, _ := kdl.ParseString(fixtureProfile)
	for _, name := range []string{"policy", "input"} {
		before, _ := kdl.EmitToString(kdl.NewDocument(old.GetNode(name)))
		after, _ := kdl.EmitToString(kdl.NewDocument(doc.GetNode(name)))
		if before != after {
			t.Fatalf("%s changed: %s", name, after)
		}
	}
	session := doc.GetNode("session").Children()
	if got := session.GetNode("control").Arg(0).String(); got != "host-admin" {
		t.Fatalf("control IPC was not enabled: %s", got)
	}
	if got := session.GetNode("window-manager").Arg(0).String(); got != filepath.Join(binaries, "hagia") {
		t.Fatal(got)
	}
	for _, component := range session.GetNodes("shell-component") {
		binary := map[string]string{"bar": "lom", "application-launcher": "bemenu-sophia"}[component.Arg(1).String()]
		if got := component.Children().GetNode("executable").Arg(0).String(); got != filepath.Join(binaries, binary) {
			t.Fatal(got)
		}
	}
	if session.GetNode("shell-component").Children().GetNode("config").Arg(0).String() != "/home/test/lom.kdl" {
		t.Fatal("changed shell config")
	}
	binds := doc.GetNode("shortcut").Children().GetNodes("bind")
	if len(binds) != 2 || binds[1].Arg(0).String() != "Super+o" || binds[1].Arg(1).String() != "policy:toggle-overview" {
		t.Fatal(result)
	}
	again, err := renderProfile(result, binaries)
	if err != nil || again != result {
		t.Fatalf("not idempotent: %v", err)
	}
}

func TestProfileRefusesAmbiguousOrUnsupportedInputs(t *testing.T) {
	cases := map[string]string{
		"duplicate control":   strings.Replace(fixtureProfile, "session {", `session { control "disabled"; control "host-admin";`, 1),
		"invalid control":     strings.Replace(fixtureProfile, "session {", `session { control #true;`, 1),
		"conflict":            strings.Replace(fixtureProfile, "shortcut {", `shortcut { bind "Super+O" "session:logout";`, 1),
		"duplicate":           strings.Replace(fixtureProfile, "shortcut {", `shortcut { bind "Super+o" "policy:toggle-overview"; bind "super+O" "policy:toggle-overview";`, 1),
		"repeated session":    fixtureProfile + "session {}\n",
		"include":             fixtureProfile + "include \"extra.kdl\"\n",
		"unknown role":        strings.Replace(fixtureProfile, `"bar"`, `"dock"`, 1),
		"missing role":        strings.Replace(fixtureProfile, `shell-component "menu"`, `/- shell-component "menu"`, 1),
		"repeated executable": strings.Replace(fixtureProfile, `executable "/old/lom";`, `executable "/old/lom"; executable "/another/lom";`, 1),
		"malformed":           "session {",
	}
	for name, source := range cases {
		t.Run(name, func(t *testing.T) {
			if _, err := renderProfile(source, "/release/bin"); err == nil {
				t.Fatal("accepted")
			}
		})
	}
}

func TestPersonalProfileEnablesPreviouslyDisabledControl(t *testing.T) {
	for _, value := range []string{"disabled", "host-admin"} {
		input := strings.Replace(fixtureProfile, "session {", `session { control "`+value+`";`, 1)
		result, err := renderProfile(input, "/release/bin")
		if err != nil {
			t.Fatal(err)
		}
		doc, _ := kdl.ParseString(result)
		controls := doc.GetNode("session").Children().GetNodes("control")
		if len(controls) != 1 || controls[0].Arg(0).String() != "host-admin" {
			t.Fatal(result)
		}
	}
}

func TestProfileSelectsNinePForBarAndLauncher(t *testing.T) {
	// A source that still names current-ipc is rewritten: new profiles are
	// 9P-only.
	source := strings.Replace(fixtureProfile, `gpu direct;`, `transport "current-ipc"; gpu direct;`, 1)
	for _, input := range []string{fixtureProfile, source} {
		result, err := renderProfile(input, "/release/bin")
		if err != nil {
			t.Fatal(err)
		}
		doc, err := kdl.ParseString(result)
		if err != nil {
			t.Fatal(err)
		}
		for _, component := range doc.GetNode("session").Children().GetNodes("shell-component") {
			if got := component.Children().GetNode("transport").Arg(0).String(); got != "9p2000.L" {
				t.Fatalf("%s wire %s, want 9p2000.L", component.Arg(1).String(), got)
			}
		}
		if err := requireNinePProfile(result); err != nil {
			t.Fatal(err)
		}
		again, err := renderProfile(result, "/release/bin")
		if err != nil || again != result {
			t.Fatalf("profile not idempotent: %v", err)
		}
	}
	for _, value := range []string{`transport #true;`, `transport "bogus";`, `transport "9p2000.L"; transport "current-ipc";`} {
		bad := strings.Replace(fixtureProfile, `gpu denied;`, value+` gpu denied;`, 1)
		if _, err := renderProfile(bad, "/release/bin"); err == nil {
			t.Fatalf("accepted %s", value)
		}
	}
}

func TestSealedProfileMustBeNinePOnly(t *testing.T) {
	good, err := renderProfile(fixtureProfile, "/release/bin")
	if err != nil {
		t.Fatal(err)
	}
	for name, profile := range map[string]string{
		"current-ipc bar":   strings.Replace(good, `transport "9p2000.L"`, `transport "current-ipc"`, 1),
		"missing transport": fixtureProfile,
		"missing launcher":  strings.Replace(fixtureProfile, `shell-component "menu"`, `/- shell-component "menu"`, 1),
	} {
		if err := requireNinePProfile(profile); err == nil {
			t.Fatalf("accepted %s", name)
		}
	}
}
