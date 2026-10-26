//! Path rules and file helpers for the session store (session-v3 §6.1).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

use crate::entry::{FileLine, SessionEntry, SessionHeader};
use crate::error::SessionError;
use crate::migrate::{FileFormat, MigrationWarning, detect, legacy::migrate_oino_legacy_text};
use crate::time::system_time_ms;

/// Name of the directory (inside the sessions root) that keeps migrated pre-v1 files.
pub const LEGACY_DIR_NAME: &str = ".legacy";

/// pi `resolvePath`: absolute (joined to the process cwd) and lexically normalised, without
/// resolving symlinks.
#[must_use]
pub fn resolve_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    let mut normalised = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalised.pop();
            }
            other => normalised.push(other),
        }
    }
    normalised
}

/// `--<cwd>--` directory name: leading `/` or `\` removed, every `/`, `\`, `:` → `-`.
#[must_use]
pub fn encode_cwd(cwd: &Path) -> String {
    let text = resolve_path(cwd).to_string_lossy().into_owned();
    let trimmed = text.strip_prefix(['/', '\\']).unwrap_or(&text);
    format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
}

/// `<sessions_root>/--<cwd>--/`.
#[must_use]
pub fn cwd_session_dir(sessions_root: &Path, cwd: &Path) -> PathBuf {
    sessions_root.join(encode_cwd(cwd))
}

/// `<ts>_<sessionId>.jsonl` for a header.
#[must_use]
pub fn session_file_name(header: &SessionHeader) -> String {
    format!("{}_{}.jsonl", header.timestamp.file_stem(), header.id)
}

/// Default agent dir: `OINO_AGENT_DIR`, else `~/.oino`.
#[must_use]
pub fn default_agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OINO_AGENT_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::home_dir().map(|home| home.join(".oino"))
}

/// Default sessions root: `<agent_dir>/sessions`.
#[must_use]
pub fn default_sessions_root() -> Option<PathBuf> {
    default_agent_dir().map(|dir| dir.join("sessions"))
}

/// pi session id rule: `^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?$`.
///
/// # Errors
///
/// [`SessionError::InvalidSessionId`] with pi's message.
pub fn validate_session_id(id: &str) -> Result<(), SessionError> {
    let bytes = id.as_bytes();
    let edge_ok = |b: Option<&u8>| b.is_some_and(u8::is_ascii_alphanumeric);
    let middle_ok = bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if edge_ok(bytes.first()) && edge_ok(bytes.last()) && middle_ok {
        Ok(())
    } else {
        Err(SessionError::InvalidSessionId(id.to_owned()))
    }
}

pub(crate) fn read_to_string(path: &Path) -> Result<String, SessionError> {
    fs::read_to_string(path).map_err(|e| SessionError::io(path, e))
}

/// First non-empty line of a file (bounded to 1 MiB like pi's header scan).
pub(crate) fn first_line(path: &Path) -> Result<String, SessionError> {
    let file = File::open(path).map_err(|e| SessionError::io(path, e))?;
    let mut reader = std::io::Read::take(BufReader::new(file), 1024 * 1024);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| SessionError::io(path, e))?;
        if read == 0 || !line.trim().is_empty() {
            return Ok(line.trim_end_matches(['\n', '\r']).to_owned());
        }
    }
}

/// Header of a pi session file, `None` when the first line is not a valid header.
pub(crate) fn read_header(path: &Path) -> Option<SessionHeader> {
    match FileLine::parse(&first_line(path).ok()?) {
        Ok(FileLine::Header(header)) => Some(header),
        _ => None,
    }
}

pub(crate) fn serialize_lines(
    header: &SessionHeader,
    entries: &[SessionEntry],
) -> Result<String, SessionError> {
    let json_error = |source| SessionError::Json {
        path: PathBuf::new(),
        line: 0,
        source,
    };
    let mut text = serde_json::to_string(header).map_err(json_error)?;
    text.push('\n');
    for entry in entries {
        text.push_str(&serde_json::to_string(entry).map_err(json_error)?);
        text.push('\n');
    }
    Ok(text)
}

fn create_parent(path: &Path) -> Result<(), SessionError> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))
        }
        _ => Ok(()),
    }
}

/// Creates `path` exclusively and writes `text` (pi `openSync(file, "wx")`).
pub(crate) fn write_new(path: &Path, text: &str) -> Result<(), SessionError> {
    create_parent(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| SessionError::io(path, e))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| SessionError::io(path, e))
}

/// Replaces `path` atomically: `<path>.tmp`, fsync, rename.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), SessionError> {
    create_parent(path)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut file = File::create(&tmp).map_err(|e| SessionError::io(&tmp, e))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| SessionError::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| SessionError::io(path, e))
}

/// Appends one line (`O_APPEND`, single write).
pub(crate) fn append_line(path: &Path, line: &str) -> Result<(), SessionError> {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| SessionError::io(path, e))?;
    let mut buffer = String::with_capacity(line.len() + 1);
    buffer.push_str(line);
    buffer.push('\n');
    file.write_all(buffer.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| SessionError::io(path, e))
}

/// Outcome of migrating one pre-v1 Oino file (D21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigratedSession {
    /// New v3 file, `None` when the session had no messages (only moved aside).
    pub path: Option<PathBuf>,
    /// Where the original was moved (`<root>/.legacy/<name>`).
    pub legacy_path: PathBuf,
    /// Dropped/approximated records.
    pub warnings: Vec<MigrationWarning>,
}

fn unique_path(dir: &Path, stem: &str, id: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}_{id}.jsonl"));
    let mut suffix = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-{suffix}_{id}.jsonl"));
        suffix += 1;
    }
    candidate
}

fn move_aside(path: &Path, sessions_root: &Path) -> Result<PathBuf, SessionError> {
    let legacy_dir = sessions_root.join(LEGACY_DIR_NAME);
    fs::create_dir_all(&legacy_dir).map_err(|e| SessionError::io(&legacy_dir, e))?;
    let name = path
        .file_name()
        .map_or_else(|| "session.jsonl".into(), std::ffi::OsStr::to_os_string);
    let mut target = legacy_dir.join(&name);
    let mut suffix = 1;
    while target.exists() {
        let mut numbered = name.clone();
        numbered.push(format!(".{suffix}"));
        target = legacy_dir.join(numbered);
        suffix += 1;
    }
    fs::rename(path, &target).map_err(|e| SessionError::io(path, e))?;
    Ok(target)
}

/// Migrates a pre-v1 Oino file to `<sessions_root>/--<cwd>--/<ts>_<id>.jsonl` and moves the
/// original to `<sessions_root>/.legacy/` (session-v3 §5.2 steps 6–7).
///
/// # Errors
///
/// I/O errors, or [`SessionError::MissingHeader`] when the file has no legacy header.
pub fn migrate_legacy_file(
    path: &Path,
    sessions_root: &Path,
) -> Result<MigratedSession, SessionError> {
    let text = read_to_string(path)?;
    let base_ms = fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map_or_else(|_| crate::time::now_ms(), system_time_ms);
    let migration = migrate_oino_legacy_text(&text, base_ms).map_err(|error| match error {
        SessionError::MissingHeader(_) => SessionError::MissingHeader(path.to_path_buf()),
        other => other,
    })?;
    let new_path = if migration.has_messages {
        let dir = cwd_session_dir(sessions_root, Path::new(&migration.header.cwd));
        let target = unique_path(
            &dir,
            &migration.header.timestamp.file_stem(),
            &migration.header.id,
        );
        write_atomic(
            &target,
            &serialize_lines(&migration.header, &migration.entries)?,
        )?;
        Some(target)
    } else {
        None
    };
    let legacy_path = move_aside(path, sessions_root)?;
    Ok(MigratedSession {
        path: new_path,
        legacy_path,
        warnings: migration.warnings,
    })
}

/// Whether the file's first line is a pre-v1 Oino record.
pub(crate) fn is_legacy_file(path: &Path) -> bool {
    first_line(path).is_ok_and(|line| matches!(detect(&line), FileFormat::OinoLegacy { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_cwd_like_pi() {
        assert_eq!(encode_cwd(Path::new("/home/oi/p")), "--home-oi-p--");
        assert_eq!(encode_cwd(Path::new("/a/./b/../c")), "--a-c--");
    }

    #[test]
    fn session_id_rule() {
        for ok in ["a", "a.b_c-d", "019fcbea-df52-7491"] {
            assert!(validate_session_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-a", "a-", "a b", "a/b"] {
            assert!(validate_session_id(bad).is_err(), "{bad}");
        }
    }
}
