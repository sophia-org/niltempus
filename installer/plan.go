package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime/debug"
	"strings"
)

func locations(config string) (Locations, error) {
	bases := make([]string, 3)
	for i, entry := range [][2]string{{"XDG_CONFIG_HOME", "~/.config"}, {"XDG_STATE_HOME", "~/.local/state"}, {"XDG_CACHE_HOME", "~/.cache"}} {
		value := os.Getenv(entry[0])
		if value == "" {
			value = entry[1]
		}
		base, err := homePath(value)
		if err != nil {
			return Locations{}, err
		}
		bases[i] = filepath.Join(base, "sophia-niltempus-desktop")
	}
	if config == "" {
		config = filepath.Join(bases[0], "config.json")
	}
	return Locations{config, bases[1], bases[2]}, nil
}

func releaseID(plan Plan) string {
	plan.ReleaseID = ""
	b, _ := json.Marshal(plan)
	return "niltempus-" + digest(b)[:20]
}

func createPlan(loc Locations) (Plan, error) {
	var config Config
	if err := readJSON(loc.Config, &config); err != nil {
		return Plan{}, err
	}
	plan, err := createSourcePlan(config)
	if err != nil {
		return Plan{}, err
	}
	repo, err := niltempusRepository(config)
	if err != nil {
		return Plan{}, err
	}
	integration, err := planIntegration(repo, plan.Sources["sophia"])
	if err != nil {
		return Plan{}, fmt.Errorf("niltempus tooling: %w", err)
	}
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return Plan{}, fmt.Errorf("niltempus installer has no Go build information")
	}
	if err := installerRevision(info, integration.Source.Commit); err != nil {
		return Plan{}, err
	}
	plan.Niltempus = &integration
	plan.ReleaseID = releaseID(plan)
	return plan, nil
}

func niltempusRepository(config Config) (Repository, error) {
	if config.Integration != (Repository{}) {
		return Repository{}, fmt.Errorf("the integration repository setting is retired; configure niltempus as the single installer and tooling source")
	}
	if config.Niltempus.Path == "" || config.Niltempus.Reference == "" {
		return Repository{}, fmt.Errorf("configure the niltempus repository path and reference explicitly")
	}
	return config.Niltempus, nil
}

func installerRevision(info *debug.BuildInfo, expected string) error {
	settings := map[string]string{}
	for _, setting := range info.Settings {
		if _, exists := settings[setting.Key]; exists {
			return fmt.Errorf("duplicate installer build setting: %s", setting.Key)
		}
		settings[setting.Key] = setting.Value
	}
	if settings["vcs"] != "git" || settings["vcs.revision"] != expected || settings["vcs.modified"] != "false" {
		return fmt.Errorf("build the installer from the clean selected niltempus revision %s with go build -buildvcs=true", expected)
	}
	return nil
}

func createSourcePlan(config Config) (Plan, error) {
	if len(config.Repositories) != len(components) {
		return Plan{}, fmt.Errorf("configure exactly sophia, hagia, narthex, lom and bemenu")
	}
	profile, err := homePath(config.Profile)
	if err != nil {
		return Plan{}, err
	}
	profile, err = filepath.EvalSymlinks(profile)
	if err != nil {
		return Plan{}, err
	}
	data, err := os.ReadFile(profile)
	if err != nil {
		return Plan{}, err
	}
	if _, err := renderProfile(string(data), "/validation/target/release"); err != nil {
		return Plan{}, err
	}
	executable, err := os.Executable()
	if err != nil {
		return Plan{}, err
	}
	installerHash, err := fileDigest(executable)
	if err != nil {
		return Plan{}, err
	}
	plan := Plan{Schema: 2, Sources: map[string]Source{}, Profile: profile, ProfileSHA256: digest(data), InstallerSHA256: installerHash}
	for _, name := range components {
		repo, ok := config.Repositories[name]
		if !ok {
			return Plan{}, fmt.Errorf("invalid repository/reference for %s", name)
		}
		source, err := resolveSource(repo)
		if err != nil {
			return Plan{}, fmt.Errorf("%s: %w", name, err)
		}
		plan.Sources[name] = source
	}
	plan.ReleaseID = releaseID(plan)
	return plan, nil
}

func resolveSource(repo Repository) (Source, error) {
	if repo.Reference == "" || strings.HasPrefix(repo.Reference, "-") {
		return Source{}, fmt.Errorf("invalid repository reference")
	}
	root, err := homePath(repo.Path)
	if err != nil {
		return Source{}, err
	}
	root, err = filepath.EvalSymlinks(root)
	if err != nil {
		return Source{}, err
	}
	commit, err := git(root, "rev-parse", "--verify", repo.Reference+"^{commit}")
	if err != nil {
		return Source{}, err
	}
	signature, err := git(root, "log", "-1", "--format=%G?", commit)
	if err != nil {
		return Source{}, err
	}
	if signature != "N" {
		if _, err := git(root, "verify-commit", commit); err != nil {
			return Source{}, err
		}
	}
	repo.Path = root
	return Source{repo, commit, signature}, nil
}
