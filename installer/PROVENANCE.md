# Installer provenance

The initial import is the unchanged `dot_local/share/sophia-niltempus-desktop`
subtree of chezmoi commit
`287f771eda99ae10294e8182593bfccb276d0ffa`, whose signature was verified before
extraction. Only that subtree was archived; personal configuration, credentials
and unrelated dotfiles were not imported.

The niltempus repository now owns this installer alongside the desktop build
and integration tools. Subsequent commits adapt their source binding and public
entrypoint to this repository. The imported README describes the previous
deployment until that adaptation lands. Existing installation, configuration
and user-state paths remain compatibility interfaces.

This import does not deploy or run the installer and does not modify the
personal WM. Earlier test-package evidence does not establish final release
readiness; the final candidate must use bound build dependencies and pass the
combined gates.
