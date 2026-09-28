package main

const prefix = "/opt/sophia-niltempus-desktop"
const desktopFile = "sophia-niltempus-desktop.desktop"

// The current-IPC entry of plan-schema-2 releases. New (schema 3) releases
// are 9P-only and never generate it; installation removes the entry when the
// selected release lacks its launcher.
const ipcDesktopFile = "sophia-niltempus-desktop-ipc.desktop"
const ipcLauncher = "bin/sophia-niltempus-desktop-ipc-session"

// The retired sealed-Hagia 9P entry; install removes its stale login file.
const retiredNineDesktopFile = "sophia-niltempus-desktop-9p.desktop"

var components = []string{"sophia", "hagia", "narthex", "lom", "bemenu"}

type Repository struct {
	Path      string `json:"path"`
	Reference string `json:"reference"`
}

type Config struct {
	Profile      string                `json:"profile"`
	Repositories map[string]Repository `json:"repositories"`
	Integration  Repository            `json:"integration"`
	Niltempus    Repository            `json:"niltempus"`
	// Explicit helper inputs for plan schema 3; no defaults.
	Inputs *PlanInputs `json:"inputs,omitempty"`
}

type Source struct {
	Repository
	Commit    string `json:"commit"`
	Signature string `json:"signature"`
}

type Plan struct {
	Schema          int               `json:"schema"`
	ReleaseID       string            `json:"release_id"`
	Sources         map[string]Source `json:"sources"`
	Profile         string            `json:"profile"`
	ProfileSHA256   string            `json:"profile_sha256"`
	InstallerSHA256 string            `json:"installer_sha256"`
	Integration     *IntegrationPlan  `json:"integration,omitempty"`
	Niltempus       *IntegrationPlan  `json:"niltempus,omitempty"`
	// Plan schema 3: the reviewed Nim dependency manifests (paths and
	// independently supplied digests) and the Hagia C SDK revision.
	Inputs *PlanInputs `json:"inputs,omitempty"`
	// Historical sealed-path releases omit this. New launches select managed
	// component paths while the base desktop release stays immutable.
	ComponentUpdates bool `json:"component_updates,omitempty"`
}

// Omitted on historical plans so their release identities stay unchanged.
// Integration is retained only to verify historical plans. New builds bind
// the installer and packaging tools to the same signed niltempus revision.
type IntegrationPlan struct {
	Source          Source `json:"source"`
	CargoHome       string `json:"cargo_home"`
	CargoLockSHA256 string `json:"cargo_lock_sha256"`
}

type FileRecord struct {
	SHA256 string `json:"sha256"`
	Mode   uint32 `json:"mode"`
}

type Manifest struct {
	Plan  Plan                  `json:"plan"`
	Files map[string]FileRecord `json:"files"`
}

type Locations struct{ Config, State, Cache string }
