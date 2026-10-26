//! Settings storage: files (locked, atomic) or memory (pi `FileSettingsStorage`,
//! `InMemorySettingsStorage`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::error::{SettingsErrorKind, SettingsScope};
use crate::paths::LOCK_SUFFIX;

/// Where settings layers live.
#[derive(Debug, Clone)]
pub(crate) enum Storage {
    /// `<agent>/settings.json` and `<cwd>/.oino/settings.json`.
    Files { global: PathBuf, project: PathBuf },
    /// In-process strings (tests, SDK `inMemory`).
    Memory {
        global: Option<String>,
        project: Option<String>,
    },
}

impl Storage {
    pub(crate) fn path(&self, scope: SettingsScope) -> Option<&Path> {
        match (self, scope) {
            (Self::Files { global, .. }, SettingsScope::Global) => Some(global),
            (Self::Files { project, .. }, SettingsScope::Project) => Some(project),
            (Self::Memory { .. }, _) => None,
        }
    }

    /// Current content of a layer; `None` when the file does not exist.
    pub(crate) fn read(&self, scope: SettingsScope) -> Result<Option<String>, SettingsErrorKind> {
        match self {
            Self::Files { .. } => {
                let Some(path) = self.path(scope) else {
                    return Ok(None);
                };
                read_optional(path).map_err(|e| SettingsErrorKind::Read(e.to_string()))
            }
            Self::Memory { global, project } => Ok(match scope {
                SettingsScope::Global => global.clone(),
                SettingsScope::Project => project.clone(),
            }),
        }
    }

    /// Locked read-modify-write: `next` maps the current content to the new content.
    /// Files are locked with an exclusive advisory lock on `<file>.lock` and replaced
    /// atomically (temp file + rename).
    pub(crate) fn update(
        &mut self,
        scope: SettingsScope,
        next: impl FnOnce(Option<&str>) -> Result<String, SettingsErrorKind>,
    ) -> Result<(), SettingsErrorKind> {
        match self {
            Self::Memory { global, project } => {
                let slot = match scope {
                    SettingsScope::Global => global,
                    SettingsScope::Project => project,
                };
                *slot = Some(next(slot.as_deref())?);
                Ok(())
            }
            Self::Files { global, project } => {
                let path = match scope {
                    SettingsScope::Global => global.as_path(),
                    SettingsScope::Project => project.as_path(),
                };
                with_file_lock(path, || {
                    let current =
                        read_optional(path).map_err(|e| SettingsErrorKind::Read(e.to_string()))?;
                    let content = next(current.as_deref())?;
                    write_atomic(path, content.as_bytes())
                        .map_err(|e| SettingsErrorKind::Write(e.to_string()))
                })
            }
        }
    }
}

/// Read a UTF-8 file, `None` when it does not exist.
pub(crate) fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Run `f` while holding an exclusive advisory lock on `<path>.lock` (parent dirs created).
///
/// # Errors
///
/// [`SettingsErrorKind::Lock`] when the lock file cannot be created or locked, or `f`'s error.
pub(crate) fn with_file_lock<T>(
    path: &Path,
    f: impl FnOnce() -> Result<T, SettingsErrorKind>,
) -> Result<T, SettingsErrorKind> {
    let lock = open_lock(path).map_err(|e| SettingsErrorKind::Lock(e.to_string()))?;
    lock.lock()
        .map_err(|e| SettingsErrorKind::Lock(e.to_string()))?;
    let result = f();
    // Closing the descriptor releases the lock even if unlock fails.
    let _ = lock.unlock();
    result
}

fn open_lock(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(LOCK_SUFFIX);
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
}

/// Write `bytes` to a sibling temp file, fsync, then rename over `path`.
///
/// # Errors
///
/// Any I/O error; the temp file is removed on failure.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
