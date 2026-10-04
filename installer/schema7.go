package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Plan schema 3 is the 9P-only desktop candidate. It packages with the
// current helper CLI and verifies the external release manifest schema 7,
// which binds Hagia's vendored C SDK. Plan schemas 1 and 2 (external
// manifest schema 6) remain readable only so that installed releases can be
// verified, re-activated when recorded, and rolled back to.
const currentPlanSchema = 3

// The external (Sophia/integration) release manifest schema that plan
// schema 3 requires. It is a different numbering from the Plan's.
const currentExternalSchema = "7"
const legacyExternalSchema = "6"

// Hagia's vendored C SDK manifest, sealed in a schema-7 release.
const sealedCSDKManifest = "share/sophia-policy/hagia/c-sdk.manifest.json"

// The activation ledger written by the integration activator under the
// install prefix: RELEASE_ID MANIFEST_SHA256 SHA256SUMS_SHA256 per line.
const activationLedger = "activated-releases"

// The reviewed dependency manifest for one Nim product and its sha256,
// supplied independently of the file (by the operator or the director).
type NimDepsInput struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}

// Explicit build inputs for the current helper CLI. There are no defaults:
// every value comes from the configuration and is bound into the plan.
type PlanInputs struct {
	HagiaNimDeps      NimDepsInput `json:"hagia_nim_deps"`
	NarthexNimDeps    NimDepsInput `json:"narthex_nim_deps"`
	HagiaCSDKRevision string       `json:"hagia_c_sdk_revision"`
	// The lock provider is a component only, never part of a release plan:
	// omitted, it leaves every plan's bytes unchanged.
	KleisNimDeps *NimDepsInput `json:"kleis_nim_deps,omitempty"`
}

func lowerHex(value string, length int) bool {
	if len(value) != length {
		return false
	}
	for _, c := range value {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}

// validateInputs refuses a missing, malformed or mismatched input before
// anything is staged. A draft dependency manifest authorizes nothing.
func validateInputs(inputs *PlanInputs, sources map[string]Source) error {
	if inputs == nil {
		return fmt.Errorf("configure inputs explicitly: the reviewed hagia and narthex Nim dependency manifests with their independently supplied sha256, and the Hagia C SDK revision (there are no defaults)")
	}
	if !lowerHex(inputs.HagiaCSDKRevision, 40) {
		return fmt.Errorf("inputs.hagia_c_sdk_revision must be 40 lowercase hex: %q", inputs.HagiaCSDKRevision)
	}
	for _, item := range []struct {
		product string
		input   NimDepsInput
	}{{"hagia", inputs.HagiaNimDeps}, {"narthex", inputs.NarthexNimDeps}} {
		if err := validateNimDeps(item.product, item.input, sources[item.product].Commit); err != nil {
			return fmt.Errorf("inputs.%s_nim_deps: %w", item.product, err)
		}
	}
	return nil
}

func validateNimDeps(product string, input NimDepsInput, commit string) error {
	if input.Path == "" || !filepath.IsAbs(input.Path) || filepath.Clean(input.Path) != input.Path {
		return fmt.Errorf("path must be absolute and clean: %q", input.Path)
	}
	if !lowerHex(input.SHA256, 64) {
		return fmt.Errorf("sha256 must be 64 lowercase hex: %q", input.SHA256)
	}
	info, err := os.Lstat(input.Path)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return fmt.Errorf("not a regular file: %s", input.Path)
	}
	data, err := os.ReadFile(input.Path)
	if err != nil {
		return err
	}
	if digest(data) != input.SHA256 {
		return fmt.Errorf("%s does not have the supplied sha256", input.Path)
	}
	return reviewedNimDeps(data, product, commit)
}

// The first two records of a reviewed manifest: the status, then the product
// and the exact source commit it was reviewed for.
func reviewedNimDeps(data []byte, product, commit string) error {
	lines := strings.SplitN(string(data), "\n", 3)
	if len(lines) < 3 {
		return fmt.Errorf("incomplete dependency manifest")
	}
	switch lines[0] {
	case "nim-deps schema=1 status=reviewed":
	case "nim-deps schema=1 status=draft":
		return fmt.Errorf("a draft dependency manifest authorizes nothing; supply the reviewed manifest")
	default:
		return fmt.Errorf("not a schema-1 nim-deps manifest")
	}
	fields := map[string]int{}
	values := map[string]string{}
	words := strings.Fields(lines[1])
	if len(words) == 0 || words[0] != "for" {
		return fmt.Errorf("dependency manifest lacks its for record")
	}
	for _, word := range words[1:] {
		key, value, ok := strings.Cut(word, "=")
		if ok {
			fields[key]++
			values[key] = value
		}
	}
	if fields["product"] != 1 || fields["source_commit"] != 1 || values["product"] != product || values["source_commit"] != commit {
		return fmt.Errorf("dependency manifest was not reviewed for %s at %s", product, commit)
	}
	return nil
}

// stageInputs copies the reviewed manifests into the private build directory
// and re-checks their digests there, so the files the helper reads cannot
// change after validation.
func stageInputs(inputs *PlanInputs, work string) (hagia, narthex string, err error) {
	dir := filepath.Join(work, "inputs")
	if err := os.Mkdir(dir, 0700); err != nil {
		return "", "", err
	}
	var paths []string
	for _, item := range []struct {
		name  string
		input NimDepsInput
	}{{"hagia", inputs.HagiaNimDeps}, {"narthex", inputs.NarthexNimDeps}} {
		target := filepath.Join(dir, item.name+"-nim-deps.manifest")
		if err := copyFile(item.input.Path, target, 0444); err != nil {
			return "", "", err
		}
		if hash, err := fileDigest(target); err != nil || hash != item.input.SHA256 {
			return "", "", fmt.Errorf("%s dependency manifest changed while staging", item.name)
		}
		paths = append(paths, target)
	}
	return paths[0], paths[1], nil
}

// The current helper CLI (integration xtask) invocations.
func prepareWMPairArgs(tool string, plan Plan, roots map[string]string, pair, buildDir, hagiaDeps, narthexDeps string) []string {
	return []string{tool, "prepare-wm-pair",
		"--hagia", roots["hagia"], plan.Sources["hagia"].Commit,
		"--narthex", roots["narthex"], plan.Sources["narthex"].Commit,
		pair,
		"--build-dir=" + buildDir,
		"--hagia-nim-deps=" + hagiaDeps, "--hagia-nim-deps-sha256=" + plan.Inputs.HagiaNimDeps.SHA256,
		"--narthex-nim-deps=" + narthexDeps, "--narthex-nim-deps-sha256=" + plan.Inputs.NarthexNimDeps.SHA256,
		"--hagia-c-sdk-rev=" + plan.Inputs.HagiaCSDKRevision}
}

func packageDesktopArgs(tool string, plan Plan, roots map[string]string, pair string, hashes [3]string, buildDir, stage string) []string {
	return []string{tool, "package-desktop",
		"--sophia-root=" + roots["sophia"], "--sophia-rev=" + plan.Sources["sophia"].Commit,
		"--wm-pair=" + pair,
		"--wm-pair-commits=" + plan.Sources["hagia"].Commit + "," + plan.Sources["narthex"].Commit,
		"--wm-pair-sha256=" + hashes[0] + "," + hashes[1],
		"--wm-pair-profile-sha256=" + hashes[2],
		"--wm-pair-c-sdk-rev=" + plan.Inputs.HagiaCSDKRevision,
		"--build-dir=" + buildDir, "--out=" + stage}
}

// externalFields maps each key of an external release manifest to all of
// its values, so that a repeated key is visible.
func externalFields(metadata []byte) map[string][]string {
	fields := map[string][]string{}
	for _, line := range strings.Split(string(metadata), "\n") {
		if key, value, ok := strings.Cut(line, "="); ok {
			fields[key] = append(fields[key], value)
		}
	}
	return fields
}

func oneField(fields map[string][]string, key string) (string, error) {
	switch len(fields[key]) {
	case 1:
		return fields[key][0], nil
	case 0:
		return "", fmt.Errorf("release manifest has no %s", key)
	default:
		return "", fmt.Errorf("release manifest repeats %s", key)
	}
}

// verifyExternalSchema7 is the current packaged-policy rule for Hagia's
// vendored C SDK, mirroring the integration verifier (schema 7): with Hagia
// included, both fields appear once and are well formed, the revision is
// the expected one, and the sealed manifest hashes to the recorded digest
// and names the recorded revision. With hagia_included=false, the fields
// and the sealed manifest are absent.
func verifyExternalSchema7(metadata []byte, root string, files map[string]FileRecord, expectedRevision string) error {
	fields := externalFields(metadata)
	schema, err := oneField(fields, "schema")
	if err != nil {
		return err
	}
	if schema != currentExternalSchema {
		return fmt.Errorf("release manifest is schema %q; this installer requires external schema %s for new candidates", schema, currentExternalSchema)
	}
	included, err := oneField(fields, "hagia_included")
	if err != nil {
		return err
	}
	_, sealed := files[sealedCSDKManifest]
	switch included {
	case "false":
		for _, key := range []string{"hagia_c_sdk_revision", "hagia_c_sdk_manifest_sha256"} {
			if len(fields[key]) != 0 {
				return fmt.Errorf("release declares hagia_included=false but records %s", key)
			}
		}
		if sealed {
			return fmt.Errorf("release declares hagia_included=false but contains %s", sealedCSDKManifest)
		}
		return nil
	case "true":
	default:
		return fmt.Errorf("release manifest has an invalid hagia_included")
	}
	revision, err := oneField(fields, "hagia_c_sdk_revision")
	if err != nil {
		return err
	}
	manifestSHA, err := oneField(fields, "hagia_c_sdk_manifest_sha256")
	if err != nil {
		return err
	}
	if !lowerHex(revision, 40) {
		return fmt.Errorf("release hagia_c_sdk_revision is malformed: %q", revision)
	}
	if !lowerHex(manifestSHA, 64) {
		return fmt.Errorf("release hagia_c_sdk_manifest_sha256 is malformed: %q", manifestSHA)
	}
	if !lowerHex(expectedRevision, 40) || revision != expectedRevision {
		return fmt.Errorf("release hagia_c_sdk_revision %s is not the plan's %s", revision, expectedRevision)
	}
	if !sealed || files[sealedCSDKManifest].SHA256 != manifestSHA {
		return fmt.Errorf("sealed %s is missing or does not hash to hagia_c_sdk_manifest_sha256", sealedCSDKManifest)
	}
	data, err := os.ReadFile(filepath.Join(root, sealedCSDKManifest))
	if err != nil {
		return err
	}
	if digest(data) != manifestSHA {
		return fmt.Errorf("sealed %s changed", sealedCSDKManifest)
	}
	var named struct {
		Revision string `json:"revision"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	if err := decoder.Decode(&named); err != nil || named.Revision != revision {
		return fmt.Errorf("sealed %s does not name revision %s", sealedCSDKManifest, revision)
	}
	return nil
}

// releaseDigests is "MANIFEST_SHA256 SHA256SUMS_SHA256" for an installed
// release, the identity the activation ledger records.
func releaseDigests(root string) (string, error) {
	var parts []string
	for _, name := range []string{"manifest", "SHA256SUMS"} {
		path := filepath.Join(root, name)
		info, err := os.Lstat(path)
		if err != nil {
			return "", err
		}
		if !info.Mode().IsRegular() {
			return "", fmt.Errorf("release has no regular %s", name)
		}
		hash, err := fileDigest(path)
		if err != nil {
			return "", err
		}
		parts = append(parts, hash)
	}
	return strings.Join(parts, " "), nil
}

// activationRecorded mirrors the integration activator's history, read-only
// (the privileged activator writes it). With a ledger, only an entry counts,
// and a recorded ID whose contents changed is refused. Without one (an
// installation that predates it), the current and previous targets count:
// the activator records exactly those once on its next run.
func activationRecorded(prefixDir, id, root string) (bool, error) {
	ledger := filepath.Join(prefixDir, activationLedger)
	info, err := os.Lstat(ledger)
	if err == nil {
		if !info.Mode().IsRegular() {
			return false, fmt.Errorf("activation ledger is not a regular file: %s", ledger)
		}
		data, err := os.ReadFile(ledger)
		if err != nil {
			return false, err
		}
		recorded := ""
		for _, line := range strings.Split(strings.TrimSuffix(string(data), "\n"), "\n") {
			if line == "" && len(data) == 0 {
				continue
			}
			fields := strings.Fields(line)
			if len(fields) != 3 || strings.Join(fields, " ") != line {
				return false, fmt.Errorf("activation ledger is malformed: %s", ledger)
			}
			if fields[0] == id {
				if recorded != "" {
					return false, fmt.Errorf("activation ledger repeats %s", id)
				}
				recorded = fields[1] + " " + fields[2]
			}
		}
		if recorded == "" {
			return false, nil
		}
		actual, err := releaseDigests(root)
		if err != nil {
			return false, err
		}
		if actual != recorded {
			return false, fmt.Errorf("release %s changed since it was activated (manifest or SHA256SUMS digest differs); refusing it", id)
		}
		return true, nil
	}
	if !os.IsNotExist(err) {
		return false, err
	}
	resolved, err := filepath.EvalSymlinks(root)
	if err != nil {
		return false, err
	}
	for _, link := range []string{"current", "previous"} {
		if target, err := filepath.EvalSymlinks(filepath.Join(prefixDir, link)); err == nil && target == resolved {
			return true, nil
		}
	}
	return false, nil
}
