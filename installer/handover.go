package main

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

type HagiaProcess struct {
	PID                                   int
	UID                                   uint32
	Start, Executable, SHA256, Checkpoint string
}

func readHagiaProcess(pid int) (HagiaProcess, error) {
	root := filepath.Join("/proc", strconv.Itoa(pid))
	info, err := os.Stat(root)
	if err != nil {
		return HagiaProcess{}, err
	}
	stat, err := os.ReadFile(filepath.Join(root, "stat"))
	if err != nil {
		return HagiaProcess{}, err
	}
	end := strings.LastIndexByte(string(stat), ')')
	if end < 0 {
		return HagiaProcess{}, fmt.Errorf("invalid process stat")
	}
	fields := strings.Fields(string(stat)[end+1:])
	if len(fields) <= 19 {
		return HagiaProcess{}, fmt.Errorf("incomplete process stat")
	}
	exe, err := os.Readlink(filepath.Join(root, "exe"))
	if err != nil {
		return HagiaProcess{}, err
	}
	hash, err := fileDigest(filepath.Join(root, "exe"))
	if err != nil {
		return HagiaProcess{}, err
	}
	env, err := os.ReadFile(filepath.Join(root, "environ"))
	if err != nil {
		return HagiaProcess{}, err
	}
	checkpoint := ""
	for _, item := range strings.Split(string(env), "\x00") {
		if value, ok := strings.CutPrefix(item, "HAGIA_POLICY_CHECKPOINT="); ok {
			checkpoint = value
		}
	}
	return HagiaProcess{pid, info.Sys().(*syscall.Stat_t).Uid, fields[19], strings.TrimSuffix(exe, " (deleted)"), hash, checkpoint}, nil
}

func hagiaProcesses() []HagiaProcess {
	entries, _ := os.ReadDir("/proc")
	var result []HagiaProcess
	for _, entry := range entries {
		pid, err := strconv.Atoi(entry.Name())
		if err != nil {
			continue
		}
		name, err := os.ReadFile(filepath.Join("/proc", entry.Name(), "comm"))
		if err != nil || strings.TrimSpace(string(name)) != "hagia" {
			continue
		}
		if process, err := readHagiaProcess(pid); err == nil {
			result = append(result, process)
		}
	}
	return result
}

func validateCheckpoint(process HagiaProcess) error {
	if !filepath.IsAbs(process.Checkpoint) {
		return fmt.Errorf("Hagia has no checkpoint; refusing reload")
	}
	info, err := os.Lstat(process.Checkpoint)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() || info.Sys().(*syscall.Stat_t).Uid != process.UID || info.Size() == 0 {
		return fmt.Errorf("Hagia checkpoint is not an owned regular nonempty file")
	}
	return nil
}

// The session owns process replacement and confirms the new WM's first commit.
func reloadHagia(binary, sophia, socket string) error {
	if !filepath.IsAbs(socket) {
		return fmt.Errorf("control IPC is unavailable; log into the updated Sophia niltempus Desktop session first")
	}
	var matches []HagiaProcess
	for _, process := range hagiaProcesses() {
		if process.UID == uint32(os.Getuid()) && process.Executable == binary {
			matches = append(matches, process)
		}
	}
	if len(matches) != 1 {
		return fmt.Errorf("expected one running user-owned Hagia, found %d; log into the updated personal session", len(matches))
	}
	old := matches[0]
	if err := validateCheckpoint(old); err != nil {
		return err
	}
	hash, err := fileDigest(binary)
	if err != nil {
		return err
	}
	current, err := readHagiaProcess(old.PID)
	if err != nil || current != old {
		return fmt.Errorf("Hagia process changed before restart")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if err := checked(exec.CommandContext(ctx, sophia, "msg", "--socket", socket, "session", "restart-wm")); err != nil {
		return fmt.Errorf("WM restart was not confirmed: %w", err)
	}
	for deadline := time.Now().Add(5 * time.Second); time.Now().Before(deadline); {
		for _, next := range hagiaProcesses() {
			if next.PID != old.PID && next.UID == old.UID && next.Executable == binary && next.SHA256 == hash && next.Checkpoint == old.Checkpoint {
				fmt.Printf("Hagia restarted through Sophia IPC: PID %d, SHA256 %s\n", next.PID, hash)
				return nil
			}
		}
		time.Sleep(100 * time.Millisecond)
	}
	return fmt.Errorf("IPC completed but the prepared Hagia process was not found; no retry was sent")
}
