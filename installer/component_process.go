package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

type ShellProcess struct {
	PID        int
	Start      string
	SHA256     string
	Socket     string
	Supervisor int
	StopSignal unix.Signal
}

// A PID namespace's init ignores default-fatal signals without a handler.
// SIGKILL from the ancestor namespace still stops it, after which Session owns
// connection retirement and respawn. Choose once; never escalate on a timer.
func componentStopSignal(status string) (unix.Signal, error) {
	init, havePID, haveCaught := false, false, false
	var caught uint64
	for _, line := range strings.Split(status, "\n") {
		key, value, _ := strings.Cut(line, ":")
		switch key {
		case "NSpid":
			ids := strings.Fields(value)
			if havePID || len(ids) == 0 {
				return 0, fmt.Errorf("invalid namespace PID record")
			}
			for _, id := range ids {
				pid, err := strconv.Atoi(id)
				if err != nil || pid <= 0 {
					return 0, fmt.Errorf("invalid namespace PID")
				}
				init = pid == 1
			}
			havePID = true
		case "SigCgt":
			var err error
			caught, err = strconv.ParseUint(strings.TrimSpace(value), 16, 64)
			if haveCaught || err != nil {
				return 0, fmt.Errorf("invalid caught-signal mask")
			}
			haveCaught = true
		}
	}
	if !havePID || !haveCaught {
		return 0, fmt.Errorf("missing process signal or namespace identity")
	}
	if init && caught&(1<<(unix.SIGTERM-1)) == 0 {
		return unix.SIGKILL, nil
	}
	return unix.SIGTERM, nil
}

func procFields(pid int) ([]string, error) {
	data, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return nil, err
	}
	end := strings.LastIndexByte(string(data), ')')
	if end < 0 {
		return nil, fmt.Errorf("invalid process stat")
	}
	fields := strings.Fields(string(data)[end+1:])
	if len(fields) < 20 {
		return nil, fmt.Errorf("incomplete process stat")
	}
	return fields, nil
}
func shellProcess(pid int, binary, sophiaHash string) (ShellProcess, error) {
	root := fmt.Sprintf("/proc/%d", pid)
	info, err := os.Stat(root)
	if err != nil {
		return ShellProcess{}, err
	}
	if info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) {
		return ShellProcess{}, fmt.Errorf("not owned")
	}
	exe, err := os.Readlink(root + "/exe")
	if err != nil {
		return ShellProcess{}, err
	}
	if strings.TrimSuffix(exe, " (deleted)") != binary {
		return ShellProcess{}, fmt.Errorf("different executable")
	}
	fields, err := procFields(pid)
	if err != nil {
		return ShellProcess{}, err
	}
	argv, err := os.ReadFile(root + "/cmdline")
	if err != nil {
		return ShellProcess{}, err
	}
	if string(argv) != binary+"\x00--serve\x00" {
		return ShellProcess{}, fmt.Errorf("different shell invocation")
	}
	data, err := os.ReadFile(root + "/environ")
	if err != nil {
		return ShellProcess{}, err
	}
	socket := ""
	count := 0
	for _, entry := range strings.Split(string(data), "\x00") {
		if strings.HasPrefix(entry, "SOPHIA_SHELL_SOCKET=") {
			return ShellProcess{}, fmt.Errorf("legacy transport")
		}
		if value, ok := strings.CutPrefix(entry, "SOPHIA_SHELL_9P_SOCKET="); ok {
			socket = value
			count++
		}
	}
	if count != 1 || !filepath.IsAbs(socket) {
		return ShellProcess{}, fmt.Errorf("no exact 9P endpoint")
	}
	hash, err := fileDigest(root + "/exe")
	if err != nil {
		return ShellProcess{}, err
	}
	parent, _ := strconv.Atoi(fields[1])
	supervisor := 0
	for depth := 0; depth < 16 && parent > 1; depth++ {
		p := fmt.Sprintf("/proc/%d", parent)
		if exe, err := os.Readlink(p + "/exe"); err == nil && filepath.Base(exe) == "sophia" {
			if h, err := fileDigest(p + "/exe"); err == nil && h == sophiaHash {
				supervisor = parent
				break
			}
		}
		f, err := procFields(parent)
		if err != nil {
			break
		}
		parent, _ = strconv.Atoi(f[1])
	}
	if supervisor == 0 {
		return ShellProcess{}, fmt.Errorf("no matching Sophia supervisor")
	}
	status, err := os.ReadFile(root + "/status")
	if err != nil {
		return ShellProcess{}, err
	}
	stop, err := componentStopSignal(string(status))
	if err != nil {
		return ShellProcess{}, err
	}
	// Don't combine observations across a PID's lifetime.
	again, err := procFields(pid)
	if err != nil || again[19] != fields[19] {
		return ShellProcess{}, fmt.Errorf("process changed")
	}
	return ShellProcess{pid, fields[19], hash, socket, supervisor, stop}, nil
}
func findShell(binary, sophiaHash string) (ShellProcess, error) {
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return ShellProcess{}, err
	}
	var found []ShellProcess
	for _, entry := range entries {
		pid, err := strconv.Atoi(entry.Name())
		if err != nil {
			continue
		}
		if p, err := shellProcess(pid, binary, sophiaHash); err == nil {
			found = append(found, p)
		}
	}
	if len(found) != 1 {
		return ShellProcess{}, fmt.Errorf("expected one managed shell at %s; found %d (a session started with sealed paths needs one new login)", binary, len(found))
	}
	return found[0], nil
}

// A pidfd pins the exact process. No name-based kills, PID-reuse race, process
// group signal or escalation: Session owns cleanup, leases and replacement.
func restartShell(loc Locations, name string, old ShellProcess, fd int, sophiaHash string) error {
	binary := componentPath(loc, name)
	current, err := shellProcess(old.PID, binary, sophiaHash)
	if err != nil || current != old {
		return fmt.Errorf("shell changed before restart; nothing signalled")
	}
	selected, err := componentSelection(loc, name)
	if err != nil {
		return err
	}
	signalName := "SIGTERM"
	if old.StopSignal == unix.SIGKILL {
		signalName = "SIGKILL (namespace init has no SIGTERM handler)"
	}
	fmt.Fprintf(os.Stderr, "Restarting %s: stopping PID %d with %s; waiting for Session replacement...\n", name, old.PID, signalName)
	if err := unix.PidfdSendSignal(fd, old.StopSignal, nil, 0); err != nil {
		return fmt.Errorf("shell stop refused: %w", err)
	}
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		p, err := findShell(binary, sophiaHash)
		if err == nil && p.PID != old.PID && p.Supervisor == old.Supervisor && p.Socket == old.Socket && p.SHA256 == selected.Current.SHA256 {
			// This confirms process replacement, not a frame or UI readiness claim.
			fmt.Printf("%s replaced by Session: PID %d, SHA256 %s\n", name, p.PID, p.SHA256)
			return nil
		}
		time.Sleep(50 * time.Millisecond)
	}
	return fmt.Errorf("replacement not observed; no retry sent; previous %s remains available with rollback-component", name)
}

func componentCommand(action, name string, loc Locations) error {
	if _, err := componentBinary(name); err != nil {
		return err
	}
	_, m, err := installedDesktop()
	if err != nil {
		return err
	}
	_, err = componentSelection(loc, name)
	if err != nil {
		return fmt.Errorf("component updates are not initialized: %w", err)
	}
	if action != "prepare-component" && name == "hagia" && !filepath.IsAbs(os.Getenv("SOPHIA_CONTROL_SOCKET")) {
		return fmt.Errorf("Hagia restart requires the active Session control socket")
	}
	var old ShellProcess
	fd := -1
	if action != "prepare-component" && name != "hagia" {
		old, err = findShell(componentPath(loc, name), m.Files["target/release/sophia"].SHA256)
		if err != nil {
			return err
		}
		fd, err = unix.PidfdOpen(old.PID, 0)
		if err != nil {
			return fmt.Errorf("safe process signalling unavailable: %w", err)
		}
		defer unix.Close(fd)
		again, err := shellProcess(old.PID, componentPath(loc, name), m.Files["target/release/sophia"].SHA256)
		if err != nil || again != old {
			return fmt.Errorf("shell changed before update")
		}
	}
	if action == "reload" || action == "prepare-component" {
		v, err := prepareComponent(loc, name)
		if err != nil {
			return err
		}
		if err := selectComponent(loc, v, filepath.Join(componentDir(loc, name), "versions", v.SHA256)); err != nil {
			return err
		}
	} else if action == "rollback-component" {
		if err := rollbackComponent(loc, name); err != nil {
			return err
		}
	}
	if action == "prepare-component" {
		fmt.Println("Prepared", name, "without restarting it")
		return nil
	}
	if name == "hagia" {
		return reloadHagia(personalHagia(loc), filepath.Join(prefix, "current/target/release/sophia"), os.Getenv("SOPHIA_CONTROL_SOCKET"))
	}
	return restartShell(loc, name, old, fd, m.Files["target/release/sophia"].SHA256)
}
