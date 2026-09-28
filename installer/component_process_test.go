package main

import (
	"encoding/json"
	"fmt"
	"golang.org/x/sys/unix"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"syscall"
	"testing"
	"time"
)

func TestMain(m *testing.M) {
	switch os.Getenv("NILTEMPUS_TEST_COMPONENT") {
	case "peer":
		for {
			time.Sleep(time.Second)
		}
	case "supervisor":
		path := os.Getenv("NILTEMPUS_TEST_BINARY")
		for i := 0; i < 2; i++ {
			child := exec.Command(path, "--serve")
			if os.Getenv("NILTEMPUS_TEST_PROTECTED") == "1" {
				child = exec.Command("/usr/bin/bwrap", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--tmpfs", "/run/user", "--unshare-user", "--unshare-pid", "--unshare-net", "--as-pid-1", "--die-with-parent", "--clearenv", "--dir", filepath.Dir(path), "--ro-bind", path, path, "--setenv", "NILTEMPUS_TEST_COMPONENT", "peer", "--setenv", "SOPHIA_SHELL_9P_SOCKET", "/tmp/component.sock", "--", path, "--serve")
			}
			child.Env = []string{"NILTEMPUS_TEST_COMPONENT=peer", "SOPHIA_SHELL_9P_SOCKET=/tmp/component.sock"}
			if err := child.Start(); err != nil {
				fmt.Fprintln(os.Stderr, err)
				os.Exit(90)
			}
			os.WriteFile(os.Getenv("NILTEMPUS_TEST_PID"), []byte(strconv.Itoa(child.Process.Pid)), 0600)
			child.Wait()
		}
		os.Exit(0)
	}
	os.Exit(m.Run())
}
func TestExactPidfdRestartObservesReplacementAndPreservesNeighbour(t *testing.T) {
	t.Run("ordinary", func(t *testing.T) { testShellReplacement(t, false) })
	t.Run("protected", func(t *testing.T) { testShellReplacement(t, true) })
}
func testShellReplacement(t *testing.T, protected bool) {
	loc := Locations{State: t.TempDir()}
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	bytes, err := os.ReadFile(self)
	if err != nil {
		t.Fatal(err)
	}
	old := ComponentVersion{Name: "lom", Source: Source{Commit: fmt.Sprintf("%040x", 1), Signature: "G"}, SHA256: digest(bytes)}
	if err := selectComponent(loc, old, self); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	supervisor := filepath.Join(dir, "sophia")
	if err := os.WriteFile(supervisor, bytes, 0755); err != nil {
		t.Fatal(err)
	}
	pidfile := filepath.Join(dir, "pid")
	cmd := exec.Command(supervisor)
	cmd.Env = []string{"NILTEMPUS_TEST_COMPONENT=supervisor", "NILTEMPUS_TEST_BINARY=" + componentPath(loc, "lom"), "NILTEMPUS_TEST_PID=" + pidfile}
	if protected {
		cmd.Env = append(cmd.Env, "NILTEMPUS_TEST_PROTECTED=1")
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL); cmd.Wait() }()
	neighbour := exec.Command("sleep", "30")
	if err := neighbour.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { neighbour.Process.Kill(); neighbour.Wait() }()
	var running ShellProcess
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		running, err = findShell(componentPath(loc, "lom"), digest(bytes))
		if err == nil {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err != nil {
		t.Fatal(err)
	}
	fd, err := unix.PidfdOpen(running.PID, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer unix.Close(fd)
	if _, err := findShell(componentPath(loc, "bemenu"), digest(bytes)); err == nil {
		t.Fatal("matched unrelated path")
	}
	if _, err := shellProcess(running.PID, componentPath(loc, "lom"), digest([]byte("other Sophia"))); err == nil {
		t.Fatal("matched wrong supervisor")
	}
	next := old
	candidate := filepath.Join(dir, "next")
	bytes = append(bytes, []byte("new component build")...)
	os.WriteFile(candidate, bytes, 0755)
	next.SHA256 = digest(bytes)
	if err := selectComponent(loc, next, candidate); err != nil {
		t.Fatal(err)
	}
	if err := restartShell(loc, "lom", running, fd, old.SHA256); err != nil {
		t.Fatal(err)
	}
	if err := neighbour.Process.Signal(syscall.Signal(0)); err != nil {
		t.Fatal("neighbour stopped", err)
	}
	// Reusing the captured identity cannot signal the successor.
	if err := restartShell(loc, "lom", running, fd, old.SHA256); err == nil {
		t.Fatal("stale restart accepted")
	}
	selection, _ := componentSelection(loc, "lom")
	encoded, _ := json.Marshal(selection)
	t.Log(string(encoded))
}
