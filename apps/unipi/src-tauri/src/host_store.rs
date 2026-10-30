//! Persisted store of paired hosts (`hosts.json` in the app data
//! dir). Written atomically (temp file + rename) so a crash mid-write
//! never corrupts the file.
//!
//! `HostInfo` fields agree with the UI's zod schema: `id`, `name`,
//! `ticket`, `paired_at` (Unix seconds), `last_path`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unipi_proto::{AppErrorDto, ProtoError};

use crate::path_kind::PathKind;

const HOSTS_FILE: &str = "hosts.json";

/// A paired host, persisted in `hosts.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostInfo {
    /// Stable identifier for this host: the iroh endpoint id (hex),
    /// derived from the pairing ticket.
    pub id: String,
    /// User-facing display name (defaults to the pairing-time device
    /// name, editable later in Settings).
    pub name: String,
    /// The iroh ticket string used to dial this host.
    pub ticket: String,
    /// Unix seconds (UTC) when pairing completed.
    pub paired_at: i64,
    /// The path kind of the most recent successful connection, if
    /// any (`null`/absent before the first successful connection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_path: Option<PathKind>,
    /// UNI-231: "local" for this PC's own host reached over `app.sock`
    /// (never persisted — added by `local_host::merge`); absent = iroh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
}

/// Reads `hosts.json` from `dir`, returning an empty list if the file
/// doesn't exist yet.
pub fn read_hosts(dir: &Path) -> Result<Vec<HostInfo>, AppErrorDto> {
    let file = dir.join(HOSTS_FILE);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            ProtoError::Internal(format!("hosts.json is corrupt: {error}")).to_app_error()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => {
            Err(ProtoError::Internal(format!("failed to read hosts.json: {error}")).to_app_error())
        }
    }
}

/// Writes `hosts` to `hosts.json` in `dir`, atomically: the new
/// content is written to a sibling temp file first, then renamed into
/// place, so a crash or power loss never leaves a half-written file.
pub fn write_hosts(dir: &Path, hosts: &[HostInfo]) -> Result<(), AppErrorDto> {
    std::fs::create_dir_all(dir).map_err(|error| {
        ProtoError::Internal(format!("failed to create app data dir: {error}")).to_app_error()
    })?;

    let file = dir.join(HOSTS_FILE);
    let tmp = tmp_path(dir, HOSTS_FILE);

    let json = serde_json::to_vec_pretty(hosts).map_err(|error| {
        ProtoError::Internal(format!("failed to serialize hosts.json: {error}")).to_app_error()
    })?;

    std::fs::write(&tmp, &json).map_err(|error| {
        ProtoError::Internal(format!("failed to write hosts.json.tmp: {error}")).to_app_error()
    })?;

    std::fs::rename(&tmp, &file).map_err(|error| {
        ProtoError::Internal(format!("failed to replace hosts.json: {error}")).to_app_error()
    })?;

    Ok(())
}

fn tmp_path(dir: &Path, file_name: &str) -> PathBuf {
    dir.join(format!("{file_name}.tmp"))
}

/// Inserts or replaces the host with the same `id`, then writes the
/// file back out atomically. Returns the full, updated list.
pub fn upsert_host(dir: &Path, host: HostInfo) -> Result<Vec<HostInfo>, AppErrorDto> {
    let mut hosts = read_hosts(dir)?;
    match hosts.iter_mut().find(|h| h.id == host.id) {
        Some(existing) => *existing = host,
        None => hosts.push(host),
    }
    write_hosts(dir, &hosts)?;
    Ok(hosts)
}

/// Removes the host with the given `id`, writes the file back out,
/// and returns whether a host was actually removed.
pub fn remove_host(dir: &Path, id: &str) -> Result<bool, AppErrorDto> {
    let mut hosts = read_hosts(dir)?;
    let before = hosts.len();
    hosts.retain(|h| h.id != id);
    let removed = hosts.len() != before;
    if removed {
        write_hosts(dir, &hosts)?;
    }
    Ok(removed)
}

/// Updates the `last_path` of the host with the given `id`, if
/// present. No-op (not an error) if the host isn't found, since the
/// connection may have outlived a `forget_host`.
pub fn record_last_path(dir: &Path, id: &str, path: PathKind) -> Result<(), AppErrorDto> {
    let mut hosts = read_hosts(dir)?;
    if let Some(host) = hosts.iter_mut().find(|h| h.id == id) {
        host.last_path = Some(path);
        write_hosts(dir, &hosts)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> HostInfo {
        HostInfo {
            id: id.to_string(),
            name: "my-pc".to_string(),
            ticket: "endpointabc".to_string(),
            paired_at: 1_700_000_000,
            last_path: None,
            transport: None,
        }
    }

    #[test]
    fn read_hosts_on_missing_file_returns_empty() {
        let dir = tempfile_dir();
        let hosts = read_hosts(dir.path()).expect("read");
        assert!(hosts.is_empty());
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile_dir();
        let hosts = vec![sample("a"), sample("b")];
        write_hosts(dir.path(), &hosts).expect("write");
        let read_back = read_hosts(dir.path()).expect("read");
        assert_eq!(read_back, hosts);
    }

    #[test]
    fn write_is_atomic_no_tmp_file_left_behind() {
        let dir = tempfile_dir();
        write_hosts(dir.path(), &[sample("a")]).expect("write");
        assert!(!dir.path().join("hosts.json.tmp").exists());
        assert!(dir.path().join("hosts.json").exists());
    }

    #[test]
    fn upsert_inserts_new_host() {
        let dir = tempfile_dir();
        let hosts = upsert_host(dir.path(), sample("a")).expect("upsert");
        assert_eq!(hosts.len(), 1);
    }

    #[test]
    fn upsert_replaces_existing_host_by_id() {
        let dir = tempfile_dir();
        upsert_host(dir.path(), sample("a")).expect("upsert");
        let mut renamed = sample("a");
        renamed.name = "renamed".to_string();
        let hosts = upsert_host(dir.path(), renamed).expect("upsert");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "renamed");
    }

    #[test]
    fn remove_host_deletes_matching_id() {
        let dir = tempfile_dir();
        upsert_host(dir.path(), sample("a")).expect("upsert");
        upsert_host(dir.path(), sample("b")).expect("upsert");
        let removed = remove_host(dir.path(), "a").expect("remove");
        assert!(removed);
        let hosts = read_hosts(dir.path()).expect("read");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, "b");
    }

    #[test]
    fn remove_host_missing_id_returns_false() {
        let dir = tempfile_dir();
        upsert_host(dir.path(), sample("a")).expect("upsert");
        let removed = remove_host(dir.path(), "nope").expect("remove");
        assert!(!removed);
    }

    #[test]
    fn record_last_path_updates_matching_host() {
        let dir = tempfile_dir();
        upsert_host(dir.path(), sample("a")).expect("upsert");
        record_last_path(dir.path(), "a", PathKind::Lan).expect("record");
        let hosts = read_hosts(dir.path()).expect("read");
        assert_eq!(hosts[0].last_path, Some(PathKind::Lan));
    }

    #[test]
    fn record_last_path_on_missing_host_is_a_noop() {
        let dir = tempfile_dir();
        record_last_path(dir.path(), "nope", PathKind::Relay).expect("record");
        let hosts = read_hosts(dir.path()).expect("read");
        assert!(hosts.is_empty());
    }

    /// Minimal self-cleaning temp dir, so we don't need the `tempfile`
    /// crate as a dependency just for tests.
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempfile_dir() -> TempDir {
        let mut dir = std::env::temp_dir();
        let unique = format!(
            "unipi-hoststore-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        dir.push(unique);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}
