package main

import (
	"fmt"
	"io/fs"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

// sessionLauncher is the default entry: the personal Hagia over the 9P2000.L
// WM wire, with Bemenu over files. Rollback selects an IPC shell profile too.
func sessionLauncher() string {
	return launcher("--wm-transport=9p2000.L", "desktop.kdl")
}

func ipcSessionLauncher() string {
	return launcher("--wm-transport=current-ipc", "desktop-ipc.kdl")
}

func launcher(transport, profile string) string {
	return `#!/bin/bash
set -euo pipefail
script=$(readlink -f "${BASH_SOURCE[0]}")
release=$(cd "$(dirname "$script")/.." && pwd)
unset SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE SOPHIA_HAGIA_PROFILE_MODE SOPHIA_DESKTOP_PROFILE_MODE
export SOPHIA_INSTALL_PREFIX=` + prefix + `
export SOPHIA_DESKTOP_PROFILE="$release/share/sophia-niltempus-desktop/` + profile + `"
wm="${XDG_STATE_HOME:-$HOME/.local/state}/sophia-niltempus-desktop/development/hagia"
if [[ ! -f "$wm" || ! -x "$wm" || ! -O "$wm" ]]; then
    echo "Personal Hagia is missing or not user-owned; run ~/sophia-niltempus-desktop install." >&2
    exit 1
fi
contract=$(timeout --kill-after=2s 10s "$wm" config check-environment-contract) || {
    echo "Personal Hagia lacks the required WM environment contract; run prepare-hagia with a compatible revision." >&2
    exit 1
}
if [[ "$contract" != '` + policyEnvironmentContract + `' ]]; then
    echo "Personal Hagia reports an incompatible WM environment contract." >&2
    exit 1
fi
export SOPHIA_HAGIA_BIN="$wm"
export SOPHIA_WM_BIN="$wm"
export SOPHIA_INSTALLED_ATTEMPT_MODE=hagia
export PATH="` + prefix + `/bin:$PATH"
exec "$release/bin/sophia-hagia-session" "--wm-process=$wm" "` + transport + `" "$@"
`
}

func desktopEntry() string {
	return "[Desktop Entry]\nName=Sophia niltempus Desktop\nComment=Hagia and Bemenu over 9P2000.L; Lom over IPC\nExec=" + prefix + "/current/bin/sophia-niltempus-desktop-session\nType=Application\nDesktopNames=Sophia\n"
}

func ipcDesktopEntry() string {
	return "[Desktop Entry]\nName=Sophia niltempus Desktop (current IPC)\nComment=The same release with WM and shells over current IPC\nExec=" + prefix + "/current/" + ipcLauncher + "\nType=Application\nDesktopNames=Sophia\n"
}

func collectFiles(root string) (map[string]FileRecord, error) {
	files := map[string]FileRecord{}
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		if !entry.Type().IsRegular() {
			return fmt.Errorf("release contains non-regular entry: %s", path)
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		if strings.ContainsAny(rel, "\n\r\\") {
			return fmt.Errorf("invalid release path: %q", rel)
		}
		if rel == "SHA256SUMS" || rel == "desktop-manifest.json" {
			return nil
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		hash, err := fileDigest(path)
		if err != nil {
			return err
		}
		files[rel] = FileRecord{hash, uint32(info.Mode().Perm())}
		return nil
	})
	return files, err
}

func checksums(files map[string]FileRecord) string {
	var out strings.Builder
	for _, path := range slices.Sorted(maps.Keys(files)) {
		fmt.Fprintf(&out, "%s  %s\n", files[path].SHA256, path)
	}
	return out.String()
}

func sealRelease(root string, plan Plan) error {
	files, err := collectFiles(root)
	if err != nil {
		return err
	}
	if err := writeFile(filepath.Join(root, "SHA256SUMS"), []byte(checksums(files)), 0644); err != nil {
		return err
	}
	return writeJSON(filepath.Join(root, "desktop-manifest.json"), Manifest{plan, files})
}

func verifyRelease(root string) (Manifest, error) {
	var manifest Manifest
	// current/previous are symlinks; entries inside a release must be regular.
	resolved, err := filepath.EvalSymlinks(root)
	if err != nil {
		return manifest, err
	}
	root = resolved
	if err := readJSON(filepath.Join(root, "desktop-manifest.json"), &manifest); err != nil {
		return manifest, err
	}
	if (manifest.Plan.Schema != 1 && manifest.Plan.Schema != 2) || manifest.Plan.ReleaseID != releaseID(manifest.Plan) {
		return manifest, fmt.Errorf("invalid desktop release identity")
	}
	for path := range manifest.Files {
		if !filepath.IsLocal(path) || filepath.Clean(path) != path || strings.ContainsAny(path, "\n\r\\") {
			return manifest, fmt.Errorf("unsafe manifest path: %q", path)
		}
	}
	actual, err := collectFiles(root)
	if err != nil {
		return manifest, err
	}
	if !maps.Equal(actual, manifest.Files) {
		return manifest, fmt.Errorf("release contents or permissions differ from manifest")
	}
	sums, err := os.ReadFile(filepath.Join(root, "SHA256SUMS"))
	if err != nil {
		return manifest, err
	}
	if string(sums) != checksums(actual) {
		return manifest, fmt.Errorf("checksum list differs from manifest")
	}
	metadata, err := os.ReadFile(filepath.Join(root, "manifest"))
	if err != nil {
		return manifest, err
	}
	identities := 0
	for _, line := range strings.Split(string(metadata), "\n") {
		if strings.HasPrefix(line, "release_id=") {
			identities++
			if line != "release_id="+manifest.Plan.ReleaseID {
				return manifest, fmt.Errorf("Sophia and desktop release identities disagree")
			}
		}
	}
	if identities != 1 {
		return manifest, fmt.Errorf("expected one release identity")
	}
	executables := []string{"target/release/sophia", "target/release/hagia", "target/release/narthex", "target/release/lom", "target/release/bemenu-sophia", "bin/sophia-niltempus-desktop-session", "bin/sophia-hagia-session", "tools/install_live_session.sh", "tools/activate_live_session_release.sh", "tools/verify_packaged_policy.sh"}
	files := []string{"share/sophia-niltempus-desktop/desktop.kdl", "share/wayland-sessions/" + desktopFile}
	if manifest.Plan.Integration != nil {
		if err := verifyIntegrationRelease(metadata, manifest.Plan, actual); err != nil {
			return manifest, err
		}
		executables = append(executables, "target/release/sophia-integration-xtask", "target/release/active-session-preflight", "tools/session/run_desktop_session.sh", "bin/sophia-session")
	}
	_, hasIPCLauncher := actual[ipcLauncher]
	_, hasIPCEntry := actual["share/wayland-sessions/"+ipcDesktopFile]
	// Old schema-1 releases predate the IPC entry. Keep them verifiable for
	// rollback; when either half exists, both must be present and usable.
	if manifest.Plan.Schema == 2 || hasIPCLauncher || hasIPCEntry {
		executables = append(executables, ipcLauncher)
		files = append(files, "share/wayland-sessions/"+ipcDesktopFile)
	}
	if manifest.Plan.Schema == 2 {
		files = append(files, "share/sophia-niltempus-desktop/desktop-ipc.kdl")
	} else if hasIPCLauncher {
		body, err := os.ReadFile(filepath.Join(root, ipcLauncher))
		if err != nil {
			return manifest, err
		}
		if strings.Contains(string(body), "desktop-ipc.kdl") {
			files = append(files, "share/sophia-niltempus-desktop/desktop-ipc.kdl")
		}
	}
	for _, name := range executables {
		if file, ok := actual[name]; !ok || file.Mode&0111 == 0 {
			return manifest, fmt.Errorf("missing executable: %s", name)
		}
	}
	for _, name := range files {
		if _, ok := actual[name]; !ok {
			return manifest, fmt.Errorf("missing release file: %s", name)
		}
	}
	return manifest, nil
}
