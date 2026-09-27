package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

const help = `niltempus [--config FILE] COMMAND

plan              Inspect local source refs/signatures (default; no git pull)
build             Build, validate and prepare a full release for installation
prepare DIRECTORY Verify and select an existing release for installation
verify DIRECTORY  Verify a built release's files and hashes
install           Install the prepared release, or build if none exists (sudo)
install DIRECTORY Install/activate a specific release for next login (sudo)
status            Inspect the selected installed release
rollback          Select the previous desktop release (sudo)
prepare-hagia     Build/preflight only Hagia into its user-owned executable path
reload-hagia      Prepare Hagia, then restart it through Sophia control IPC

The niltempus repository owns the installer and desktop tooling.
Existing sophia-niltempus-desktop configuration and state paths are retained.
reload-hagia restarts only the WM.
`

func printJSON(value any) error {
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(value)
}

func run(args []string) error {
	config := ""
	if len(args) > 0 && args[0] == "--config" {
		if len(args) < 3 {
			return fmt.Errorf("--config requires a path and command")
		}
		config, args = args[1], args[2:]
	}
	loc, err := locations(config)
	if err != nil {
		return err
	}
	if len(args) == 0 {
		args = []string{"plan"}
	}
	if len(args) == 1 {
		switch args[0] {
		case "help", "--help", "-h":
			fmt.Print(help)
			return nil
		case "plan":
			plan, err := createPlan(loc)
			if err != nil {
				return err
			}
			return printJSON(plan)
		case "build", "install":
			// One build owns the shared Cargo/Nim caches at a time.
			unlock, err := buildLock(loc)
			if err != nil {
				return err
			}
			defer unlock()
			if args[0] == "install" {
				path, err := preparedRelease(loc)
				if err == nil {
					return installRelease(path, loc)
				}
				// Only an absent selection allows a first build. A selected but
				// missing or damaged release must fail before invoking sudo.
				if !os.IsNotExist(err) {
					return err
				}
				if _, markerErr := os.Lstat(filepath.Join(loc.State, "prepared.json")); !os.IsNotExist(markerErr) {
					return err
				}
			}
			plan, err := createPlan(loc)
			if err != nil {
				return err
			}
			path, err := buildRelease(plan, loc)
			if err != nil {
				return err
			}
			if err := selectPreparedRelease(path, loc); err != nil {
				return err
			}
			fmt.Printf("Built and verified: %s\n", path)
			if args[0] == "install" {
				return installRelease(path, loc)
			}
			return nil
		case "rollback":
			unlock, err := buildLock(loc)
			if err != nil {
				return err
			}
			defer unlock()
			return rollback(loc)
		case "prepare-hagia", "reload-hagia":
			if args[0] == "reload-hagia" && !filepath.IsAbs(os.Getenv("SOPHIA_CONTROL_SOCKET")) {
				return fmt.Errorf("control IPC is unavailable; log into the updated Sophia niltempus Desktop session first")
			}
			unlock, err := buildLock(loc)
			if err != nil {
				return err
			}
			defer unlock()
			binary, err := prepareHagia(loc)
			if err != nil {
				return err
			}
			fmt.Printf("Personal Hagia ready: %s\n", binary)
			if args[0] == "prepare-hagia" {
				return nil
			}
			return reloadHagia(binary, filepath.Join(prefix, "current/target/release/sophia"), os.Getenv("SOPHIA_CONTROL_SOCKET"))
		case "status":
			current := filepath.Join(prefix, "current")
			if _, err := os.Lstat(current); os.IsNotExist(err) {
				fmt.Println("No Sophia niltempus Desktop release is installed.")
				return nil
			}
			manifest, err := verifyRelease(current)
			if err != nil {
				return err
			}
			return printJSON(manifest.Plan)
		}
	}
	if len(args) == 2 {
		switch args[0] {
		case "prepare":
			unlock, err := buildLock(loc)
			if err != nil {
				return err
			}
			defer unlock()
			if err := selectPreparedRelease(args[1], loc); err != nil {
				return err
			}
			fmt.Println("Release prepared. Run niltempus install.")
			return nil
		case "install":
			unlock, err := buildLock(loc)
			if err != nil {
				return err
			}
			defer unlock()
			return installRelease(args[1], loc)
		case "verify":
			_, err := verifyRelease(args[1])
			if err == nil {
				fmt.Println("Release verified.")
			}
			return err
		}
	}
	return fmt.Errorf("unknown arguments; run --help")
}

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "niltempus:", err)
		os.Exit(1)
	}
}
