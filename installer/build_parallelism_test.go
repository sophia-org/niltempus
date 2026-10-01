package main

import (
	"os"
	"runtime"
	"strconv"
	"strings"
	"testing"
)

func TestBuildParallelismInheritsCallerAndDefaultsToAvailableCPUs(t *testing.T) {
	t.Setenv("CARGO_BUILD_JOBS", "")
	if err := os.Unsetenv("CARGO_BUILD_JOBS"); err != nil {
		t.Fatal(err)
	}
	jobs, err := buildJobs()
	if err != nil || jobs != strconv.Itoa(runtime.NumCPU()) {
		t.Fatalf("default jobs=%q: %v", jobs, err)
	}
	if strings.Contains(strings.Join(buildEnvironment(nil), "\n"), "CARGO_BUILD_JOBS=") {
		t.Fatal("environment imposes a job count")
	}
	for _, value := range []string{"1", "32", "100"} {
		t.Setenv("CARGO_BUILD_JOBS", value)
		jobs, err := buildJobs()
		if err != nil || jobs != value {
			t.Fatalf("override %q: %q %v", value, jobs, err)
		}
	}
	for _, value := range []string{"", "0", "01", "-1", "+2", "2x", "999999999999999999999"} {
		t.Setenv("CARGO_BUILD_JOBS", value)
		if _, err := buildJobs(); err == nil {
			t.Fatalf("accepted %q", value)
		}
		if cmd := isolated(t.TempDir(), nil, "cargo", "build"); cmd.Err == nil {
			t.Fatalf("isolated build accepted %q", value)
		}
	}
	t.Setenv("CARGO_BUILD_JOBS", "3")
	cmd := isolated(t.TempDir(), nil, "cargo", "build", "--offline")
	for _, arg := range cmd.Args {
		if arg == "nice" || arg == "--jobs" {
			t.Fatalf("build overrides caller priority or jobs: %q", cmd.Args)
		}
	}
}
