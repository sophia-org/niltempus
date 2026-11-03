//! Stage untrusted output on boot-scoped tmpfs, preserving the existing live path.
//!
//! Only a clean wrapper shutdown permits persistence. Earlier failures leave the
//! named source and symlink intact, including external TERM/KILL of the runner.
//! Snapshot that source before sealing refusal evidence, after checking writers
//! have stopped; it is lost on reboot. This avoids synchronous disk writes but
//! neither proves the cause of a past stall nor eliminates scheduling delays.
//! Copy failures retain the RAM source; an over-cap `.partial` is deliberately
//! truncated and must never be mistaken for complete evidence.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const TMPFS_MAGIC: i64 = 0x0102_1994;

pub(super) struct SessionLog {
    file: File,
    path: PathBuf,
    target: PathBuf,
    link_identity: (u64, u64),
    cap: u64,
}

impl SessionLog {
    pub(super) fn new(path: &Path, cap: u64) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for existing in [path.to_owned(), path.with_extension("storage.json")] {
            match fs::symlink_metadata(&existing) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => {
                    return Err(format!(
                        "session log artifact already exists: {}",
                        existing.display()
                    ));
                }
            }
        }
        // /dev/shm may be removed by elogind RemoveIPC at the last PAM exit.
        let temporary = File::open("/tmp").map_err(|e| e.to_string())?;
        if rustix::fs::fstatfs(&temporary)
            .map_err(|e| e.to_string())?
            .f_type as i64
            != TMPFS_MAGIC
        {
            return Err("/tmp is not tmpfs".into());
        }
        let parent = path.parent().ok_or("session log has no parent")?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| e.to_string())?;
        // create_dir refuses a stale name; never reuse another attempt's source.
        let directory = PathBuf::from(format!(
            "/tmp/sophia-session-log-{}-{}-{}",
            rustix::process::getuid().as_raw(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|e| e.to_string())?;
        let target = directory.join("output.log");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .map_err(|e| e.to_string())?;
        let source = file.metadata().map_err(|e| e.to_string())?;
        let owner = rustix::process::getuid().as_raw();
        let directory_metadata = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if source.uid() != owner
            || source.mode() & 0o7777 != 0o600
            || directory_metadata.uid() != owner
            || directory_metadata.mode() & 0o7777 != 0o700
            || rustix::fs::fstatfs(&file)
                .map_err(|e| e.to_string())?
                .f_type as i64
                != TMPFS_MAGIC
        {
            return Err("session log source is not private owned tmpfs".into());
        }
        // Write this before launch so abnormal exits remain discoverable. It is
        // a location receipt, not a claim that the eventual bytes are complete.
        let receipt = serde_json::json!({"schema":1,"storage":"tmpfs","source":target,"source_device":source.dev(),"source_inode":source.ino(),"source_uid":source.uid(),"live_path":path,"cap_bytes":cap,"lifetime":"until reboot or explicit removal","failure":"retain source; stop writers before snapshot; do not qualify"});
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.with_extension("storage.json"))
            .map_err(|e| e.to_string())?
            .write_all(&serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink(&target, path).map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            path: path.to_owned(),
            target,
            link_identity: (meta.dev(), meta.ino()),
            cap,
        })
    }

    fn own_link(&self) -> Result<(), String> {
        let source = fs::symlink_metadata(&self.target).map_err(|e| e.to_string())?;
        let held = self.file.metadata().map_err(|e| e.to_string())?;
        if !source.is_file()
            || (source.dev(), source.ino(), source.uid()) != (held.dev(), held.ino(), held.uid())
            || source.mode() & 0o7777 != 0o600
        {
            return Err("temporary session log source was replaced or changed permissions".into());
        }
        let meta = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !meta.is_symlink()
            || (meta.dev(), meta.ino()) != self.link_identity
            || fs::read_link(&self.path).map_err(|e| e.to_string())? != self.target
        {
            return Err("temporary session log was replaced".into());
        }
        Ok(())
    }

    /// The caller has observed wrapper exit, dropped it and verified recovery.
    pub(super) fn persist(mut self) -> Result<(), String> {
        let partial = self.path.with_extension("log.partial");
        let mut save = || -> Result<(), String> {
            self.own_link()?;
            let size = self.file.metadata().map_err(|e| e.to_string())?.len();
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&partial)
                .map_err(|e| e.to_string())?;
            self.file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let copied = std::io::copy(&mut (&mut self.file).take(self.cap), &mut output)
                .map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            if size > self.cap
                || copied != size
                || self.file.metadata().map_err(|e| e.to_string())?.len() != size
            {
                return Err(format!(
                    "session log size {size} exceeds cap {} or changed during persistence",
                    self.cap
                ));
            }
            self.own_link()?;
            fs::rename(&partial, &self.path).map_err(|e| e.to_string())?;
            fs::remove_file(&self.target).map_err(|e| e.to_string())?;
            fs::remove_dir(self.target.parent().unwrap()).map_err(|e| e.to_string())
        };
        save().map_err(|e| {
            format!(
                "session log persistence failed: {e}; inspect source {} and partial {}",
                self.target.display(),
                partial.display()
            )
        })
    }
}

pub(super) fn move_new(from: &Path, to: &Path) -> Result<(), String> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        from,
        rustix::fs::CWD,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
}

#[cfg(test)]
#[path = "../../tests/support/memory_session_log.rs"]
mod tests;
