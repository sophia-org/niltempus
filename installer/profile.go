package main

import (
	"fmt"
	kdl "github.com/calico32/kdl-go"
	"path/filepath"
	"strings"
)

func uniqueNode(doc *kdl.Document, name string) (*kdl.Node, error) {
	nodes := doc.GetNodes(name)
	if len(nodes) != 1 {
		return nil, fmt.Errorf("profile requires exactly one %s node", name)
	}
	return nodes[0], nil
}

func setExecutable(doc *kdl.Document, name, path string) error {
	node, err := uniqueNode(doc, name)
	if err != nil {
		return err
	}
	if len(node.Arguments()) != 1 || node.Arg(0).Kind() != kdl.String {
		return fmt.Errorf("%s requires one executable path", name)
	}
	node.SetArg(0, kdl.NewString(path))
	return nil
}

func renderDevelopmentProfile(source, binary string) (string, error) {
	doc, err := kdl.ParseString(source, kdl.WithVersion(kdl.Version2), kdl.WithDuplicateProperties(kdl.DupError))
	if err != nil {
		return "", err
	}
	session, err := uniqueNode(doc, "session")
	if err != nil {
		return "", err
	}
	if err := setExecutable(session.Children(), "window-manager", binary); err != nil {
		return "", err
	}
	return kdl.EmitToString(doc, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
}

func renderProfile(source, binaries string) (string, error) {
	return renderTransportProfile(source, binaries, "9p2000.L")
}

func renderIPCProfile(source, binaries string) (string, error) {
	return renderTransportProfile(source, binaries, "current-ipc")
}

func renderTransportProfile(source, binaries, launcherTransport string) (string, error) {
	doc, err := kdl.ParseString(source, kdl.WithVersion(kdl.Version2), kdl.WithDuplicateProperties(kdl.DupError))
	if err != nil {
		return "", err
	}
	if len(doc.GetNodes("include")) > 0 {
		return "", fmt.Errorf("flatten profile includes with sophia config print-effective before building")
	}
	session, err := uniqueNode(doc, "session")
	if err != nil {
		return "", err
	}
	if err := setExecutable(session.Children(), "window-manager", filepath.Join(binaries, "hagia")); err != nil {
		return "", err
	}
	// This personal desktop explicitly admits host scripting for WM reloads.
	controls := session.Children().GetNodes("control")
	if len(controls) > 1 {
		return "", fmt.Errorf("duplicate session control setting")
	}
	if len(controls) == 0 {
		session.AddChild(kdl.NewNode("control", kdl.NewString("host-admin")))
	} else {
		control := controls[0]
		if len(control.Arguments()) != 1 || control.Arg(0).Kind() != kdl.String || (control.Arg(0).String() != "disabled" && control.Arg(0).String() != "host-admin") {
			return "", fmt.Errorf("invalid session control setting")
		}
		control.SetArg(0, kdl.NewString("host-admin"))
	}
	roles := map[string]bool{}
	for _, component := range session.Children().GetNodes("shell-component") {
		if len(component.Arguments()) != 2 || component.Arg(1).Kind() != kdl.String {
			return "", fmt.Errorf("shell-component requires name and role")
		}
		role := component.Arg(1).String()
		binary, ok := map[string]string{"bar": "lom", "application-launcher": "bemenu-sophia"}[role]
		if !ok || roles[role] {
			return "", fmt.Errorf("unsupported or repeated shell role %s", role)
		}
		roles[role] = true
		if err := setExecutable(component.Children(), "executable", filepath.Join(binaries, binary)); err != nil {
			return "", err
		}
		wire := "current-ipc" // Lom has not adopted the file SDK yet.
		if role == "application-launcher" {
			wire = launcherTransport
		}
		transports := component.Children().GetNodes("transport")
		if len(transports) > 1 {
			return "", fmt.Errorf("duplicate transport for %s", role)
		}
		if len(transports) == 0 {
			component.AddChild(kdl.NewNode("transport", kdl.NewString(wire)))
		} else {
			node := transports[0]
			if len(node.Arguments()) != 1 || node.Arg(0).Kind() != kdl.String || (node.Arg(0).String() != "current-ipc" && node.Arg(0).String() != "9p2000.L") {
				return "", fmt.Errorf("invalid transport for %s", role)
			}
			node.SetArg(0, kdl.NewString(wire))
		}
	}
	if len(roles) != 2 {
		return "", fmt.Errorf("this desktop requires one Lom bar and one Bemenu application-launcher")
	}
	shortcuts, err := uniqueNode(doc, "shortcut")
	if err != nil {
		return "", err
	}
	bindings := 0
	for _, bind := range shortcuts.Children().GetNodes("bind") {
		if bind.Arg(0).Kind() == kdl.String && strings.EqualFold(bind.Arg(0).String(), "Super+o") {
			bindings++
			if len(bind.Arguments()) != 2 || bind.Arg(1).Kind() != kdl.String || bind.Arg(1).String() != "policy:toggle-overview" {
				return "", fmt.Errorf("Super+O has a conflicting binding in the source profile")
			}
		}
	}
	if bindings > 1 {
		return "", fmt.Errorf("duplicate Super+O bindings in source profile")
	}
	if bindings == 0 {
		shortcuts.AddChild(kdl.NewNode("bind", kdl.NewString("Super+o"), kdl.NewString("policy:toggle-overview")))
	}
	return kdl.EmitToString(doc, kdl.WithVersion(kdl.Version2), kdl.WithIndent("    "))
}
