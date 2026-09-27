package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const policyEnvironmentContract = "hagia_environment_contract schema=1 wm_policy=sophia-wm-policy-v1 names=SOPHIA_WM_POLICY_CHECKPOINT,SOPHIA_WM_POLICY_CANDIDATE,SOPHIA_WM_POLICY_PROFILE_ACTIVATION legacy=HAGIA_POLICY_CHECKPOINT,HAGIA_POLICY_CANDIDATE,HAGIA_POLICY_PROFILE_ACTIVATION precedence=presence"

// Hagia's command line belongs to this desktop integration. Sophia only
// passes a private policy fragment to an explicitly selected checker.
func policyAdapter(hagia string) string {
	quoted := "'" + strings.ReplaceAll(hagia, "'", "'\"'\"'") + "'"
	return "#!/bin/sh\nset -eu\ntest \"$#\" = 1\nexec " + quoted + " config check --config=\"$1\"\n"
}

func preflightProfile(root, work, name, sophia, hagia, profile string) error {
	probe := isolated(root, nil, "timeout", "--kill-after=2s", "10s", hagia, "config", "check-environment-contract")
	output, err := probe.Output()
	if err != nil || string(output) != policyEnvironmentContract+"\n" {
		return fmt.Errorf("Hagia does not confirm the required WM environment contract: %v", err)
	}
	adapter := filepath.Join(work, name+"-policy-checker")
	if err := writeFile(adapter, []byte(policyAdapter(hagia)), 0700); err != nil {
		return err
	}
	log := filepath.Join(work, name+"-preflight.log")
	if err := logged(isolated(root, nil, sophia, "config", "check-session-profile", "--desktop-profile="+profile, "--default-wm="+hagia, "--policy-checker="+adapter), log); err != nil {
		return err
	}
	data, err := os.ReadFile(log)
	if err != nil {
		return err
	}
	return requireValidatedPolicy(string(data))
}

func requireValidatedPolicy(output string) error {
	verdicts := 0
	for _, line := range strings.Split(output, "\n") {
		if strings.HasPrefix(line, "sophia_session_profile_preflight ") {
			if line != "sophia_session_profile_preflight schema=1 status=accepted policy=validated" {
				return fmt.Errorf("desktop preflight did not validate the WM policy: %s", line)
			}
			verdicts++
		}
	}
	if verdicts != 1 {
		return fmt.Errorf("desktop preflight requires exactly one validated policy verdict, got %d", verdicts)
	}
	return nil
}
