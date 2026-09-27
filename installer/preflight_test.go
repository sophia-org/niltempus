package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestPolicyVerdictMustBeValidatedExactlyOnce(t *testing.T) {
	valid := "sophia_session_profile_preflight schema=1 status=accepted policy=validated"
	for _, output := range []string{"", strings.ReplaceAll(valid, "validated", "deferred"), valid + "\n" + valid, valid + " extra=true", "prefix " + valid} {
		if err := requireValidatedPolicy(output); err == nil {
			t.Fatalf("accepted missing or ambiguous policy validation: %q", output)
		}
	}
	if err := requireValidatedPolicy("log heading\n" + valid + "\nlog end\n"); err != nil {
		t.Fatal(err)
	}
}

func TestPolicyAdapterPreservesPathsAndExitStatus(t *testing.T) {
	dir := t.TempDir()
	hagia := filepath.Join(dir, "wm ' with $(literal) spaces")
	if err := os.WriteFile(hagia, []byte("#!/bin/sh\ntest \"$#\" = 3 || exit 90\ntest \"$1\" = config || exit 91\ntest \"$2\" = check || exit 92\ntest \"$3\" = \"--config=$EXPECTED_POLICY\" || exit 93\nexit 7\n"), 0700); err != nil {
		t.Fatal(err)
	}
	adapter := filepath.Join(dir, "adapter")
	if err := os.WriteFile(adapter, []byte(policyAdapter(hagia)), 0700); err != nil {
		t.Fatal(err)
	}
	policy := filepath.Join(dir, "private ' policy.kdl")
	cmd := exec.Command(adapter, policy)
	cmd.Env = append(os.Environ(), "EXPECTED_POLICY="+policy)
	if output, err := cmd.CombinedOutput(); err == nil || cmd.ProcessState.ExitCode() != 7 {
		t.Fatalf("adapter changed arguments or exit status: %v: %s", err, output)
	}
}
