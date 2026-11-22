//! Local atomic replacement and environment-owned mutation guarantees.

use std::io::Write;
use std::path::Path;

use futures::{FutureExt, future::BoxFuture};

use super::{FsError, LocalEnv};

/// The guarantee actually provided by a successful mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationGuarantee {
    /// Sibling temp file and atomic replacement of the destination.
    AtomicReplace,
    /// The client owns the write; its protocol does not promise atomic replacement.
    ClientManaged,
}

impl MutationGuarantee {
    /// User-visible disclosure when the host cannot guarantee atomicity.
    #[must_use]
    pub fn notice(self) -> &'static str {
        match self {
            Self::AtomicReplace => "",
            Self::ClientManaged => {
                "\nThe filesystem client performed this write; its protocol does not guarantee atomic replacement."
            }
        }
    }
}

/// Environment-owned mutation capability. Implementations must preserve filesystem
/// routing. They must not silently fall back to direct host writes after a client error.
pub trait AtomicFileMutation: Send + Sync {
    /// Replace a file, checking an optional previously observed snapshot before mutation.
    /// Unsupported environments fail explicitly without writing. `ClientManaged` must
    /// be disclosed to the caller rather than represented as guaranteed atomicity.
    fn write_file_atomic<'a>(
        &'a self,
        path: &'a str,
        _data: &'a [u8],
        _expected: Option<&'a [u8]>,
    ) -> BoxFuture<'a, Result<MutationGuarantee, FsError>> {
        async move {
            let mut error = FsError::new("ENOTSUP", "atomic_write", Some(path));
            "this execution environment cannot guarantee atomic file mutation"
                .clone_into(&mut error.description);
            Err(error)
        }
        .boxed()
    }
}

// Kept here rather than local.rs so filesystem capability changes are independent
// of concurrent process/Job Object work in that file.
impl AtomicFileMutation for LocalEnv {
    fn write_file_atomic<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
        expected: Option<&'a [u8]>,
    ) -> BoxFuture<'a, Result<MutationGuarantee, FsError>> {
        let path = path.to_owned();
        let data = data.to_vec();
        let expected = expected.map(<[u8]>::to_vec);
        async move {
            let result = tokio::task::spawn_blocking(move || {
                atomic_write_sync(Path::new(&path), &data, expected.as_deref())
            })
            .await
            .map_err(|error| {
                let mut fs_error = FsError::new("EIO", "atomic_write", None);
                fs_error.description = error.to_string();
                fs_error
            })?;
            result.map_err(|error| {
                let mut fs_error = FsError::from_io(&error, "atomic_write", None);
                fs_error.description = error.to_string();
                fs_error
            })?;
            Ok(MutationGuarantee::AtomicReplace)
        }
        .boxed()
    }
}

fn atomic_write_sync(path: &Path, bytes: &[u8], expected: Option<&[u8]>) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("File has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // Follow symlinks to preserve the link itself, like ordinary file writes.
    let target = if path.exists() {
        std::fs::canonicalize(path)?
    } else {
        path.to_owned()
    };
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("File has no parent directory"))?;
    let temp = parent.join(format!(".oino-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        if let Ok(metadata) = std::fs::metadata(&target) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Some(expected) = expected
            && std::fs::read(&target)? != expected
        {
            return Err(std::io::Error::other(format!(
                "File \"{}\" has been modified externally since it was last read. Please use the Read tool to read the file before editing it again.",
                path.display()
            )));
        }
        replace(&temp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> std::io::Result<()> {
    // Rust's Windows rename uses MoveFileExW(MOVEFILE_REPLACE_EXISTING). Retry only
    // sharing/lock violations, never remove the destination first.
    for attempt in 0..8_u32 {
        match std::fs::rename(source, target) {
            Ok(()) => return Ok(()),
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) && attempt < 7 => {
                std::thread::sleep(std::time::Duration::from_millis(10 * (1 << attempt)));
            }
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("Atomic replace retry exhausted"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_replace_refuses_changed_snapshot_and_cleans_temp() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, b"external change").unwrap();
        let error = atomic_write_sync(&path, b"replacement", Some(b"previous read")).unwrap_err();
        assert!(error.to_string().contains("modified externally"));
        assert_eq!(std::fs::read(&path).unwrap(), b"external change");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
