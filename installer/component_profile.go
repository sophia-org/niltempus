package main

import (
	"fmt"
	kdl "github.com/calico32/kdl-go"
	"os"
	"path/filepath"
	"strings"
)

func renderComponentProfile(source string, loc Locations) (string, error) {
	doc, err := kdl.ParseString(source, kdl.WithVersion(kdl.Version2), kdl.WithDuplicateProperties(kdl.DupError))
	if err != nil {
		return "", err
	}
	session, err := uniqueNode(doc, "session")
	if err != nil {
		return "", err
	}
	if err := setExecutable(session.Children(), "window-manager", componentPath(loc, "hagia")); err != nil {
		return "", err
	}
	seen := map[string]bool{}
	for _, node := range session.Children().GetNodes("shell-component") {
		if len(node.Arguments()) != 2 || node.Arg(1).Kind() != kdl.String {
			return "", fmt.Errorf("invalid shell component")
		}
		name, ok := map[string]string{"bar": "lom", "application-launcher": "bemenu"}[node.Arg(1).String()]
		if !ok || seen[name] {
			return "", fmt.Errorf("unsupported or duplicate shell component")
		}
		seen[name] = true
		if err := setExecutable(node.Children(), "executable", componentPath(loc, name)); err != nil {
			return "", err
		}
	}
	if len(seen) != 2 {
		return "", fmt.Errorf("expected bar and application launcher")
	}
	// The lock provider is optional; when the profile names one, it is kleis.
	if err := setLockProvider(session, componentPath(loc, "kleis"), false); err != nil {
		return "", err
	}
	return kdl.EmitToString(doc, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
}
func componentProfile(release string, loc Locations) (string, error) {
	if _, err := verifyRelease(release); err != nil {
		return "", err
	}
	for _, name := range []string{"hagia", "lom", "bemenu"} {
		if _, err := componentSelection(loc, name); err != nil {
			return "", fmt.Errorf("%s: %w", name, err)
		}
	}
	data, err := os.ReadFile(filepath.Join(release, "share/sophia-niltempus-desktop/desktop.kdl"))
	if err != nil {
		return "", err
	}
	rendered, err := renderComponentProfile(string(data), loc)
	if err != nil {
		return "", err
	}
	// A profile that names a lock provider needs a selected kleis: a missing
	// executable would leave the session's provider unstarted for good.
	if strings.Contains(rendered, componentPath(loc, "kleis")) {
		if _, err := componentSelection(loc, "kleis"); err != nil {
			return "", fmt.Errorf("kleis: %w", err)
		}
	}
	return rendered, nil
}
