package main

import (
	"bufio"
	"context"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func TestPersonalHagiaPublicationIsAtomicAndVerified(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	binary := personalHagia(loc)
	if err := writeFile(binary, []byte("old"), 0755); err != nil {
		t.Fatal(err)
	}
	old, err := os.Open(binary)
	if err != nil {
		t.Fatal(err)
	}
	defer old.Close()
	candidate := filepath.Join(loc.State, "candidate")
	if err := writeFile(candidate, []byte("new"), 0755); err != nil {
		t.Fatal(err)
	}
	source := Source{Commit: "fixture"}
	if err := publishPersonalHagia(candidate, source, digest([]byte("wrong")), loc); err == nil {
		t.Fatal("accepted changed candidate")
	}
	if data, _ := os.ReadFile(binary); string(data) != "old" {
		t.Fatal("refusal changed executable")
	}
	if err := publishPersonalHagia(candidate, source, digest([]byte("new")), loc); err != nil {
		t.Fatal(err)
	}
	if data, _ := io.ReadAll(old); string(data) != "old" {
		t.Fatal("old inode changed")
	}
	if data, _ := os.ReadFile(binary); string(data) != "new" {
		t.Fatal("new binary absent")
	}
	before, _ := os.Stat(binary)
	if err := publishPersonalHagia(candidate, source, digest([]byte("new")), loc); err != nil {
		t.Fatal(err)
	}
	after, _ := os.Stat(binary)
	if !os.SameFile(before, after) {
		t.Fatal("unchanged binary replaced")
	}
	var metadata DevelopmentHagia
	if err := readJSON(filepath.Join(filepath.Dir(binary), "hagia.json"), &metadata); err != nil || metadata.Source != source || metadata.SHA256 != digest([]byte("new")) {
		t.Fatalf("metadata: %v %+v", err, metadata)
	}
}

func TestPersonalHagiaRefusesSymlinkDestination(t *testing.T) {
	loc := Locations{State: t.TempDir()}
	candidate := filepath.Join(loc.State, "candidate")
	if err := writeFile(candidate, []byte("new"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(personalHagia(loc)), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(candidate, personalHagia(loc)); err != nil {
		t.Fatal(err)
	}
	if err := publishPersonalHagia(candidate, Source{}, digest([]byte("new")), loc); err == nil {
		t.Fatal("followed symlink")
	}
}

func TestReloadRequiresExplicitControlEndpoint(t *testing.T) {
	if err := reloadHagia("/unused/hagia", "/must-not-run/sophia", ""); err == nil || !strings.Contains(err.Error(), "control IPC is unavailable") {
		t.Fatal(err)
	}
}

// Real private processes prove executable replacement and exact restart-command
// forwarding. The Sophia CLI is a fixture; real protocol settlement is Sophia's.
func TestPrivateIPCReplacementRetainsPreparedUserBinary(t *testing.T) {
	if _, err := exec.LookPath("cc"); err != nil {
		t.Skip("C compiler unavailable")
	}
	loc := Locations{State: t.TempDir()}
	code := `#include <signal.h>
#include <stdio.h>
#include <unistd.h>
static volatile sig_atomic_t stop;
static void restart(int sig) { (void)sig; stop=1; }
int main(void) { signal(SIGTERM,restart); puts(VERSION); fflush(stdout); while(!stop) pause(); return 0; }
`
	source := filepath.Join(loc.State, "fixture.c")
	if err := writeFile(source, []byte(code), 0600); err != nil {
		t.Fatal(err)
	}
	binary, candidate := personalHagia(loc), filepath.Join(loc.State, "candidate")
	if err := os.MkdirAll(filepath.Dir(binary), 0700); err != nil {
		t.Fatal(err)
	}
	for _, build := range [][2]string{{binary, "old"}, {candidate, "new"}} {
		if out, err := exec.Command("cc", source, "-DVERSION=\""+build[1]+"\"", "-o", build[0]).CombinedOutput(); err != nil {
			t.Fatalf("%v: %s", err, out)
		}
	}
	checkpoint := filepath.Join(loc.State, "checkpoint")
	if err := writeFile(checkpoint, []byte("fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	start := func() (*exec.Cmd, error) {
		cmd := exec.CommandContext(ctx, binary)
		cmd.Env = []string{"HAGIA_POLICY_CHECKPOINT=" + checkpoint}
		out, err := cmd.StdoutPipe()
		if err != nil {
			return nil, err
		}
		if err := cmd.Start(); err != nil {
			return nil, err
		}
		_, err = bufio.NewReader(out).ReadString('\n')
		return cmd, err
	}
	first, err := start()
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	restarted := make(chan *exec.Cmd, 1)
	restartErr := make(chan error, 1)
	go func() {
		defer close(done)
		if err := first.Wait(); err != nil {
			restartErr <- err
			return
		}
		next, err := start()
		if err != nil {
			restartErr <- err
			return
		}
		restarted <- next
		_ = next.Wait()
	}()
	defer func() { cancel(); <-done }()
	hash, _ := fileDigest(candidate)
	if err := publishPersonalHagia(candidate, Source{}, hash, loc); err != nil {
		t.Fatal(err)
	}
	cli := filepath.Join(loc.State, "sophia")
	trace := filepath.Join(loc.State, "arguments")
	t.Setenv("CONTROL_TRACE", trace)
	t.Setenv("CONTROL_TEST_PID", strconv.Itoa(first.Process.Pid))
	if err := writeFile(cli, []byte("#!/bin/sh\nexit 1\n"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := reloadHagia(binary, cli, "/private/test.sock"); err == nil {
		t.Fatal("CLI refusal reported success")
	}
	if old, err := readHagiaProcess(first.Process.Pid); err != nil || old.SHA256 == hash {
		t.Fatalf("refusal changed live fixture: %v", err)
	}
	stub := "#!/bin/sh\nprintf '%s\\n' \"$@\" >\"$CONTROL_TRACE\"\nkill -TERM \"$CONTROL_TEST_PID\"\n"
	if err := writeFile(cli, []byte(stub), 0755); err != nil {
		t.Fatal(err)
	}
	if err := reloadHagia(binary, cli, "/private/test.sock"); err != nil {
		t.Fatal(err)
	}
	if args, _ := os.ReadFile(trace); string(args) != "msg\n--socket\n/private/test.sock\nsession\nrestart-wm\n" {
		t.Fatalf("wrong IPC request: %s", args)
	}
	select {
	case next := <-restarted:
		live, err := readHagiaProcess(next.Process.Pid)
		if err != nil || live.SHA256 != hash {
			t.Fatalf("replacement not retained: %v", err)
		}
	case err := <-restartErr:
		t.Fatal(err)
	default:
		t.Fatal("replacement not started")
	}
	if current, _ := fileDigest(binary); current != hash {
		t.Fatal("user-owned binary was reverted")
	}
}

func TestDevelopmentBuildRemovesInheritedHagiaAuthority(t *testing.T) {
	env := strings.Join(buildEnvironment([]string{"HAGIA_POLICY_CHECKPOINT=/live", "HAGIA_POLICY_SOCKET=/live/socket", "DBUS_SESSION_BUS_ADDRESS=live", "PATH=/usr/bin"}), "\n")
	if strings.Contains(env, "HAGIA_") || strings.Contains(env, "DBUS_SESSION_BUS_ADDRESS") {
		t.Fatal(env)
	}
}
