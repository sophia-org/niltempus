package main

const prefix = "/opt/sophia-niltempus-desktop"
const desktopFile = "sophia-niltempus-desktop.desktop"

// The default entry runs the WM over 9P2000.L. This entry is its explicit
// current-IPC rollback, with the same personal Hagia.
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
}

// Omitted on historical plans so their release identities stay unchanged.
// New builds require the signed external packager and its accepted offline cache.
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
