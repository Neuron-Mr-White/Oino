//! Node-style file system and spawn errors.

use std::fmt;
use std::io;

/// A file system error rendered like Node: `{code}: {description}, {syscall} '{path}'`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct FsError {
    /// Errno name, e.g. `ENOENT`.
    pub code: String,
    /// libuv description, e.g. `no such file or directory`.
    pub description: String,
    /// Failing call, e.g. `open`.
    pub syscall: String,
    /// Path the call was made with (Node omits it for `read`).
    pub path: Option<String>,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}, {}", self.code, self.description, self.syscall)?;
        if let Some(path) = &self.path {
            write!(f, " '{path}'")?;
        }
        Ok(())
    }
}

impl FsError {
    /// An error with an explicit errno name.
    #[must_use]
    pub fn new(code: &str, syscall: &str, path: Option<&str>) -> Self {
        Self {
            code: code.to_owned(),
            description: describe(code).to_owned(),
            syscall: syscall.to_owned(),
            path: path.map(str::to_owned),
        }
    }

    /// Converts an I/O error raised by `syscall` on `path`.
    #[must_use]
    pub fn from_io(error: &io::Error, syscall: &str, path: Option<&str>) -> Self {
        Self::new(errno_name(error), syscall, path)
    }
}

/// Failure to start a process (Node `spawn {program} {code}`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("spawn {program} {code}")]
pub struct SpawnError {
    /// Program that failed to start.
    pub program: String,
    /// Errno name.
    pub code: String,
}

impl SpawnError {
    /// Converts an I/O error raised while starting `program`.
    #[must_use]
    pub fn from_io(error: &io::Error, program: &str) -> Self {
        Self {
            program: program.to_owned(),
            code: errno_name(error).to_owned(),
        }
    }
}

/// The errno name of an I/O error (`UNKNOWN` when the OS code is not recognised).
fn errno_name(error: &io::Error) -> &'static str {
    #[cfg(unix)]
    if let Some(name) = error.raw_os_error().and_then(unix_errno_name) {
        return name;
    }
    match error.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        io::ErrorKind::DirectoryNotEmpty => "ENOTEMPTY",
        io::ErrorKind::ReadOnlyFilesystem => "EROFS",
        io::ErrorKind::StorageFull => "ENOSPC",
        io::ErrorKind::InvalidFilename => "ENAMETOOLONG",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::ResourceBusy => "EBUSY",
        _ => "UNKNOWN",
    }
}

#[cfg(unix)]
fn unix_errno_name(raw: i32) -> Option<&'static str> {
    use nix::errno::Errno;
    let name = match Errno::from_raw(raw) {
        Errno::ENOENT => "ENOENT",
        Errno::EACCES => "EACCES",
        Errno::EEXIST => "EEXIST",
        Errno::EISDIR => "EISDIR",
        Errno::ENOTDIR => "ENOTDIR",
        Errno::ENOTEMPTY => "ENOTEMPTY",
        Errno::EPERM => "EPERM",
        Errno::ENOSPC => "ENOSPC",
        Errno::EROFS => "EROFS",
        Errno::ELOOP => "ELOOP",
        Errno::ENAMETOOLONG => "ENAMETOOLONG",
        Errno::EBUSY => "EBUSY",
        Errno::EMFILE => "EMFILE",
        Errno::EINVAL => "EINVAL",
        Errno::EIO => "EIO",
        Errno::EXDEV => "EXDEV",
        Errno::ETXTBSY => "ETXTBSY",
        Errno::EAGAIN => "EAGAIN",
        Errno::EBADF => "EBADF",
        _ => return None,
    };
    Some(name)
}

/// libuv `uv_strerror` texts for the codes above.
fn describe(code: &str) -> &'static str {
    match code {
        "ENOENT" => "no such file or directory",
        "EACCES" => "permission denied",
        "EEXIST" => "file already exists",
        "EISDIR" => "illegal operation on a directory",
        "ENOTDIR" => "not a directory",
        "ENOTEMPTY" => "directory not empty",
        "EPERM" => "operation not permitted",
        "ENOSPC" => "no space left on device",
        "EROFS" => "read-only file system",
        "ELOOP" => "too many symbolic links encountered",
        "ENAMETOOLONG" => "name too long",
        "EBUSY" => "resource busy or locked",
        "EMFILE" => "too many open files",
        "EINVAL" => "invalid argument",
        "EIO" => "i/o error",
        "EXDEV" => "cross-device link not permitted",
        "ETXTBSY" => "text file is busy",
        "EAGAIN" => "resource temporarily unavailable",
        "EBADF" => "bad file descriptor",
        _ => "unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_like_node() {
        let error = FsError::new("ENOENT", "open", Some("/x"));
        assert_eq!(
            error.to_string(),
            "ENOENT: no such file or directory, open '/x'"
        );
        let error = FsError::new("EISDIR", "read", None);
        assert_eq!(
            error.to_string(),
            "EISDIR: illegal operation on a directory, read"
        );
    }

    #[test]
    fn maps_io_kinds() {
        let error = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(FsError::from_io(&error, "stat", None).code, "ENOENT");
    }
}
