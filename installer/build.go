package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

// Apply the mode in the child, without changing this process's global umask.
// In particular, Git must not create group-writable policy inputs.
func buildCommand(name string, args ...string) *exec.Cmd {
	argv := append([]string{"-c", `umask 022; exec "$@"`, "desktop-build", name}, args...)
	return exec.Command("sh", argv...)
}

// No inherited installed binaries, smoke flags, display sockets or DRM devices
// may enter a build or profile validation command.
func buildEnvironment(environ []string) []string {
	var result []string
	blocked := map[string]bool{"DISPLAY": true, "WAYLAND_DISPLAY": true, "WAYLAND_SOCKET": true, "XAUTHORITY": true, "DBUS_SESSION_BUS_ADDRESS": true, "CARGO_TARGET_DIR": true, "RUSTFLAGS": true, "CARGO_ENCODED_RUSTFLAGS": true, "XDG_CONFIG_HOME": true, "XDG_RUNTIME_DIR": true, "TMPDIR": true, "CARGO_NET_OFFLINE": true}
	for _, item := range environ {
		name, _, _ := strings.Cut(item, "=")
		if !strings.HasPrefix(name, "SOPHIA_") && !strings.HasPrefix(name, "HAGIA_") && !blocked[name] {
			result = append(result, item)
		}
	}
	return append(result, "XDG_CONFIG_HOME=/tmp/config", "XDG_RUNTIME_DIR=/tmp/runtime", "TMPDIR=/tmp", "CARGO_NET_OFFLINE=true")
}

// Make shares Cargo's explicit job override. With none, both use the
// available CPUs; never turn an invalid override into an unbounded make -j.
func buildJobs() (string, error) {
	value, set := os.LookupEnv("CARGO_BUILD_JOBS")
	if !set {
		return strconv.Itoa(runtime.NumCPU()), nil
	}
	n, err := strconv.Atoi(value)
	if err != nil || n <= 0 || strconv.Itoa(n) != value {
		return "", fmt.Errorf("CARGO_BUILD_JOBS must be a positive integer, not %q", value)
	}
	return value, nil
}

func isolated(root string, extra map[string]string, command ...string) *exec.Cmd {
	args := []string{"--die-with-parent", "--unshare-pid", "--unshare-net", "--bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/run/user", "--tmpfs", "/tmp", "--dir", "/tmp/config", "--dir", "/tmp/runtime", "--", "sh", "-c", `umask 022; exec "$@"`, "desktop-build"}
	cmd := exec.Command("bwrap", append(args, command...)...)
	cmd.Dir, cmd.Env = root, buildEnvironment(os.Environ())
	if _, err := buildJobs(); err != nil {
		cmd.Err = err
	}
	for name, value := range extra {
		cmd.Env = append(cmd.Env, name+"="+value)
	}
	return cmd
}

func logged(cmd *exec.Cmd, path string) error {
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()
	fmt.Fprintf(f, "%q\n", cmd.Args)
	fmt.Fprintf(os.Stderr, "Log: %s\n", path)
	cmd.Stdout, cmd.Stderr = f, f
	start := time.Now()
	fmt.Fprintf(f, "Started: %s\n", start.UTC().Format(time.RFC3339Nano))
	err = cmd.Run()
	elapsed := time.Since(start).Round(time.Millisecond)
	status := "PASS"
	if err != nil {
		status = "FAIL"
	}
	fmt.Fprintf(f, "Finished: %s; elapsed=%s; status=%s\n", time.Now().UTC().Format(time.RFC3339Nano), elapsed, status)
	fmt.Fprintf(os.Stderr, "%s: %s (%s)\n", filepath.Base(path), status, elapsed)
	if err != nil {
		return fmt.Errorf("command failed: %w; see %s", err, path)
	}
	return nil
}

func buildRelease(plan Plan, loc Locations) (string, error) {
	destination := filepath.Join(loc.State, "releases", plan.ReleaseID)
	if _, err := os.Stat(destination); err == nil {
		_, err = verifyRelease(destination)
		return destination, err
	}
	if plan.Niltempus == nil || plan.Integration != nil {
		return "", fmt.Errorf("new builds require the single niltempus source; the external integration packager setting is retired; rerun plan")
	}
	// Missing, malformed or mismatched helper inputs stop here, before any
	// directory is created or source staged.
	if plan.Schema != currentPlanSchema {
		return "", fmt.Errorf("new builds require plan schema %d (9P-only); rerun plan", currentPlanSchema)
	}
	if err := validateInputs(plan.Inputs, plan.Sources); err != nil {
		return "", err
	}
	if !plan.ComponentUpdates {
		return "", fmt.Errorf("new builds require component update support; rerun plan")
	}
	profile, err := os.ReadFile(plan.Profile)
	if err != nil {
		return "", err
	}
	if digest(profile) != plan.ProfileSHA256 {
		return "", fmt.Errorf("profile changed after planning")
	}
	for _, dir := range []string{filepath.Join(loc.State, "builds"), filepath.Dir(destination), loc.Cache} {
		if err := os.MkdirAll(dir, 0700); err != nil {
			return "", err
		}
	}
	work, err := os.MkdirTemp(filepath.Join(loc.State, "builds"), plan.ReleaseID+"-")
	if err != nil {
		return "", err
	}
	fmt.Fprintf(os.Stderr, "Build plan and logs: %s\n", work)
	if err := writeJSON(filepath.Join(work, "plan.json"), plan); err != nil {
		return "", err
	}
	if err := writeFile(filepath.Join(work, "source-desktop.kdl"), profile, 0600); err != nil {
		return "", err
	}
	roots := map[string]string{}
	for _, name := range components {
		source := plan.Sources[name]
		root, err := buildSource(name, source, loc.Cache, work)
		if err != nil {
			return "", err
		}
		roots[name] = root
	}
	integrationRoot, err := buildSource("niltempus", plan.Niltempus.Source, loc.Cache, work)
	if err != nil {
		return "", err
	}
	roots["integration"] = integrationRoot
	if err := writeJSON(filepath.Join(work, "source-paths.json"), roots); err != nil {
		return "", err
	}
	for _, name := range []string{"lom"} {
		target := filepath.Join(loc.Cache, name+"-target")
		if err := os.MkdirAll(target, 0755); err != nil {
			return "", err
		}
		if err := linkBuildTarget(roots[name], target); err != nil {
			return "", err
		}
	}
	// Check the independent client cache before compiling the desktop. Fetch
	// remains offline; provisioning missing dependencies is a separate action.
	if err := logged(isolated(roots["lom"], nil, "cargo", "fetch", "--locked", "--offline"), filepath.Join(work, "check-lom-dependencies.log")); err != nil {
		return "", fmt.Errorf("provision Lom's locked Cargo dependencies before building: %w", err)
	}
	stage, err := packageDesktop(plan, roots, work, loc.Cache)
	if err != nil {
		return "", err
	}
	if err := logged(isolated(roots["lom"], nil, "cargo", "build", "--locked", "--offline", "--release"), filepath.Join(work, "build-lom.log")); err != nil {
		return "", err
	}
	jobs, err := buildJobs()
	if err != nil {
		return "", err
	}
	if err := logged(isolated(roots["bemenu"], nil, "make", "-j"+jobs, "bemenu-sophia", "EXTRA_WARNINGS=-Werror", "GIT_SHA1="+plan.Sources["bemenu"].Commit, "GIT_TAG="+plan.Sources["bemenu"].Commit), filepath.Join(work, "build-bemenu.log")); err != nil {
		return "", err
	}
	for _, pair := range [][2]string{{filepath.Join(roots["lom"], "target/release/lom"), "target/release/lom"}, {filepath.Join(roots["bemenu"], "bemenu-sophia"), "target/release/bemenu-sophia"}} {
		if err := copyFile(pair[0], filepath.Join(stage, pair[1]), 0755); err != nil {
			return "", err
		}
	}
	// The activator and its ledger library come from the selected niltempus
	// revision; the activator sources both libraries.
	for _, tool := range []struct {
		name string
		mode os.FileMode
	}{{"install_live_session.sh", 0755}, {"activate_live_session_release.sh", 0755}, {"lib/live_session_surface.sh", 0755}, {"lib/activation_ledger.sh", 0644}} {
		if err := copyFile(filepath.Join(roots["integration"], "tools", tool.name), filepath.Join(stage, "tools", tool.name), tool.mode); err != nil {
			return "", err
		}
	}
	// One 9P-only profile and one login entry; no current-IPC variant.
	installed := filepath.Join(prefix, "releases", plan.ReleaseID)
	finalProfile, err := renderProfile(string(profile), filepath.Join(installed, "target/release"))
	if err != nil {
		return "", err
	}
	if err := writeFile(filepath.Join(stage, "share/sophia-niltempus-desktop/desktop.kdl"), []byte(finalProfile), 0644); err != nil {
		return "", err
	}
	validation, err := renderProfile(string(profile), filepath.Join(stage, "target/release"))
	if err != nil {
		return "", err
	}
	validationPath := filepath.Join(work, "desktop-validation.kdl")
	if err := writeFile(validationPath, []byte(validation), 0600); err != nil {
		return "", err
	}
	if err := preflightProfile(stage, work, "desktop", filepath.Join(stage, "target/release/sophia"), filepath.Join(stage, "target/release/hagia"), validationPath); err != nil {
		return "", err
	}
	if err := writeFile(filepath.Join(stage, "bin/sophia-niltempus-desktop-session"), []byte(sessionLauncher()), 0755); err != nil {
		return "", err
	}
	self, err := os.Executable()
	if err != nil {
		return "", err
	}
	if err := copyFile(self, filepath.Join(stage, "target/release/niltempus"), 0755); err != nil {
		return "", err
	}
	if err := writeFile(filepath.Join(stage, "share/wayland-sessions", desktopFile), []byte(desktopEntry()), 0644); err != nil {
		return "", err
	}
	metadataPath := filepath.Join(stage, "manifest")
	metadata, err := os.ReadFile(metadataPath)
	if err != nil {
		return "", err
	}
	lines := strings.Split(string(metadata), "\n")
	for i, line := range lines {
		if strings.HasPrefix(line, "release_id=") {
			lines[i] = "release_id=" + plan.ReleaseID
		}
	}
	if err := writeFile(metadataPath, []byte(strings.Join(lines, "\n")), 0644); err != nil {
		return "", err
	}
	for name, root := range roots {
		source := plan.Sources[name]
		if name == "integration" {
			source = plan.Niltempus.Source
		}
		if head, err := git(root, "rev-parse", "HEAD"); err != nil || head != source.Commit {
			return "", fmt.Errorf("build source identity changed: %s", name)
		}
		if _, err := git(root, "diff", "--exit-code", "HEAD", "--"); err != nil {
			return "", fmt.Errorf("build changed tracked source: %w", err)
		}
	}
	if hash, err := fileDigest(plan.Profile); err != nil || hash != plan.ProfileSHA256 {
		return "", fmt.Errorf("source profile changed during build; rerun build")
	}
	if err := sealRelease(stage, plan); err != nil {
		return "", err
	}
	if _, err := verifyRelease(stage); err != nil {
		return "", err
	}
	if err := os.Rename(stage, destination); err != nil {
		return "", err
	}
	return destination, nil
}
