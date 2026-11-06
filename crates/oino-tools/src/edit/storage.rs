//! Encoding-preserving atomic mutation and session-scoped observed file state.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use oino_agent::ToolError;

use crate::ToolConfig;

#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// Text decoded without its BOM, with the original encoding and newline convention.
pub(crate) struct TextFile {
    pub(crate) text: String,
    encoding: Encoding,
    bom: bool,
    crlf: bool,
}

impl TextFile {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ToolError> {
        let (encoding, bom, body) = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
            (Encoding::Utf16Le, true, body)
        } else if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
            (Encoding::Utf16Be, true, body)
        } else if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
            (Encoding::Utf8, true, body)
        } else {
            (Encoding::Utf8, false, bytes)
        };
        let text = match encoding {
            Encoding::Utf8 => String::from_utf8(body.to_vec())
                .map_err(|_| ToolError::new("File is not valid UTF-8 or BOM-marked UTF-16"))?,
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if !body.len().is_multiple_of(2) {
                    return Err(ToolError::new("Invalid UTF-16 file length"));
                }
                let units: Vec<u16> = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| {
                        if matches!(encoding, Encoding::Utf16Le) {
                            u16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_be_bytes([pair[0], pair[1]])
                        }
                    })
                    .collect();
                String::from_utf16(&units).map_err(|_| ToolError::new("Invalid UTF-16 file"))?
            }
        };
        let crlf = text.contains("\r\n");
        Ok(Self {
            text,
            encoding,
            bom,
            crlf,
        })
    }

    pub(crate) fn encode(&self, text: &str, trailing_newline: bool) -> Vec<u8> {
        let mut text = text.replace("\r\n", "\n");
        if trailing_newline && !text.ends_with('\n') {
            text.push('\n');
        }
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        let mut bytes = Vec::new();
        match self.encoding {
            Encoding::Utf8 => {
                if self.bom {
                    bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
                }
                bytes.extend_from_slice(text.as_bytes());
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let little = matches!(self.encoding, Encoding::Utf16Le);
                if self.bom {
                    bytes.extend_from_slice(if little { &[0xff, 0xfe] } else { &[0xfe, 0xff] });
                }
                for unit in text.encode_utf16() {
                    bytes.extend_from_slice(&if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
            }
        }
        bytes
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    length: usize,
    hash: u64,
}

fn fingerprint(bytes: &[u8]) -> Fingerprint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Fingerprint {
        length: bytes.len(),
        hash: hasher.finish(),
    }
}

type Observations = HashMap<(String, String), Fingerprint>;
static OBSERVED: OnceLock<Mutex<Observations>> = OnceLock::new();

fn observation_key(config: &ToolConfig, path: &str) -> (String, String) {
    let session = config.context.session_env().map_or_else(
        || format!("{}:{}", config.agent_dir, config.cwd),
        |session| format!("{}:{}", config.agent_dir, session.session_id),
    );
    (session, path.to_owned())
}

pub(crate) fn observe(config: &ToolConfig, path: &str, bytes: &[u8]) -> Result<(), ToolError> {
    OBSERVED
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| ToolError::new("File observation lock poisoned"))?
        .insert(observation_key(config, path), fingerprint(bytes));
    Ok(())
}

pub(crate) fn check_observed(
    config: &ToolConfig,
    path: &str,
    bytes: &[u8],
) -> Result<(), ToolError> {
    let mut observed = OBSERVED
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| ToolError::new("File observation lock poisoned"))?;
    let previous = observed
        .entry(observation_key(config, path))
        .or_insert_with(|| fingerprint(bytes));
    if *previous != fingerprint(bytes) {
        return Err(ToolError::new(format!(
            "File \"{path}\" has been modified externally since it was last read. Please use the Read tool to read the file before editing it again."
        )));
    }
    Ok(())
}

/// Local mutation is deliberately refused on environments without an atomic replace API.
/// Never silently degrade a remote edit to a truncating write.
pub(crate) async fn atomic_write(
    config: &ToolConfig,
    path: &str,
    bytes: &[u8],
    expected: Option<&[u8]>,
) -> Result<(), ToolError> {
    if !config.env.is_local() {
        return Err(ToolError::new(
            "Atomic writes are not supported by this execution environment",
        ));
    }
    let path = path.to_owned();
    let bytes = bytes.to_vec();
    let expected = expected.map(<[u8]>::to_vec);
    tokio::task::spawn_blocking(move || {
        atomic_write_sync(Path::new(&path), &bytes, expected.as_deref())
    })
    .await
    .map_err(|error| ToolError::new(error.to_string()))?
    .map_err(|error| ToolError::new(error.to_string()))
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
