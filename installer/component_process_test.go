package main

import (
	"encoding/json"
	"fmt"
	"golang.org/x/sys/unix"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
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
	t.Run("ordinary", func(t *testing.T) { testShellReplacement(t, false, false) })
	t.Run("protected", func(t *testing.T) { testShellReplacement(t, true, false) })
	t.Run("protected_without_term_handler", func(t *testing.T) { testShellReplacement(t, true, true) })
}
func testShellReplacement(t *testing.T, protected, defaultTERM bool) {
	loc := Locations{State: t.TempDir()}
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	bytes, err := os.ReadFile(self)
	if err != nil {
		t.Fatal(err)
	}
	supervisorHash := digest(bytes)
	dir := t.TempDir()
	supervisor := filepath.Join(dir, "sophia")
	if err := os.WriteFile(supervisor, bytes, 0755); err != nil {
		t.Fatal(err)
	}
	peer := self
	if defaultTERM {
		// Go installs a SIGTERM handler; this native peer deliberately doesn't,
		// matching Lom under --as-pid-1 and exposing the original timeout.
		peer = filepath.Join(dir, "default-term-peer")
		compile := exec.Command("cc", "-Wall", "-Werror", "-x", "c", "-", "-o", peer)
		compile.Stdin = strings.NewReader("#include <unistd.h>\nint main(void) { for (;;) pause(); }\n")
		if output, err := compile.CombinedOutput(); err != nil {
			t.Fatalf("native fixture: %v: %s", err, output)
		}
		bytes, err = os.ReadFile(peer)
		if err != nil {
			t.Fatal(err)
		}
	}
	old := ComponentVersion{Name: "lom", Source: Source{Commit: fmt.Sprintf("%040x", 1), Signature: "G"}, SHA256: digest(bytes)}
	if err := selectComponent(loc, old, peer); err != nil {
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
		running, err = findShell(componentPath(loc, "lom"), supervisorHash)
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
	if defaultTERM {
		if running.StopSignal != unix.SIGKILL {
			t.Fatal("namespace init without a handler needs SIGKILL")
		}
		if err := unix.PidfdSendSignal(fd, unix.SIGTERM, nil, 0); err != nil {
			t.Fatal(err)
		}
		ready := []unix.PollFd{{Fd: int32(fd), Events: unix.POLLIN}}
		if n, err := unix.Poll(ready, 100); err != nil || n != 0 {
			t.Fatalf("control: default SIGTERM unexpectedly stopped namespace init: %d %v", n, err)
		}
	}
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
	if err := restartShell(loc, "lom", running, fd, supervisorHash); err != nil {
		t.Fatal(err)
	}
	if err := neighbour.Process.Signal(syscall.Signal(0)); err != nil {
		t.Fatal("neighbour stopped", err)
	}
	// Reusing the captured identity cannot signal the successor.
	if err := restartShell(loc, "lom", running, fd, supervisorHash); err == nil {
		t.Fatal("stale restart accepted")
	}
	selection, _ := componentSelection(loc, "lom")
	encoded, _ := json.Marshal(selection)
	t.Log(string(encoded))
}

func TestComponentStopSignal(t *testing.T) {
	for _, c := range []struct {
		status string
		signal unix.Signal
	}{
		{"NSpid:\t42\t1\nSigCgt:\t0\n", unix.SIGKILL},
		{"NSpid:\t42\t1\nSigCgt:\t4000\n", unix.SIGTERM},
		{"NSpid:\t42\nSigCgt:\t0\n", unix.SIGTERM},
	} {
		got, err := componentStopSignal(c.status)
		if err != nil || got != c.signal {
			t.Fatalf("%q: %v %v", c.status, got, err)
		}
	}
	for _, status := range []string{"", "NSpid: 1\n", "NSpid: 1\nSigCgt: x\n", "NSpid: 0\nSigCgt: 0\n", "NSpid: 1\nNSpid: 2\nSigCgt: 0\n"} {
		if _, err := componentStopSignal(status); err == nil {
			t.Fatalf("accepted %q", status)
		}
	}
}
