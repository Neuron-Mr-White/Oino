//! The device allowlist persisted in `devices.json`: phones that
//! completed pairing and are allowed to connect without re-pairing.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::EndpointId;
use serde::{Deserialize, Serialize};

/// One paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// The phone's iroh endpoint id, hex-encoded (matches
    /// `unipi_proto::HostId`'s hex form).
    pub id: String,
    /// Display name, supplied at pairing time (or a default).
    pub name: String,
    /// Unix seconds when pairing completed.
    pub paired_at: u64,
    /// Unix seconds of the most recent successful connection, if
    /// any.
    pub last_seen: Option<u64>,
}

/// The full on-disk device store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct DevicesFile {
    #[serde(default)]
    devices: Vec<Device>,
}

/// In-memory view of `devices.json`, with disk persistence on every
/// mutation (no caching across process restarts beyond what's on
/// disk).
#[derive(Debug, Clone)]
pub struct DeviceStore {
    path: PathBuf,
}

impl DeviceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        DeviceStore { path: path.into() }
    }

    fn load(&self) -> anyhow::Result<DevicesFile> {
        let devices = crate::data_dir::read_json(&self.path)?.unwrap_or_default();
        Ok(DevicesFile { devices })
    }

    fn save(&self, file: &DevicesFile) -> anyhow::Result<()> {
        crate::data_dir::write_json(&self.path, &file.devices)
    }

    /// Returns all paired devices.
    pub fn list(&self) -> anyhow::Result<Vec<Device>> {
        Ok(self.load()?.devices)
    }

    /// Returns `true` if `id` is on the allowlist.
    pub fn is_paired(&self, id: &EndpointId) -> anyhow::Result<bool> {
        let id_str = id.to_string();
        Ok(self.load()?.devices.iter().any(|d| d.id == id_str))
    }

    /// Adds a newly-paired device. Replaces any existing entry with
    /// the same id (re-pairing updates the name and `paired_at`).
    pub fn add(&self, id: &EndpointId, name: String) -> anyhow::Result<Device> {
        let mut file = self.load()?;
        let id_str = id.to_string();
        file.devices.retain(|d| d.id != id_str);
        let device = Device {
            id: id_str,
            name,
            paired_at: now_unix(),
            last_seen: None,
        };
        file.devices.push(device.clone());
        self.save(&file)?;
        Ok(device)
    }

    /// Removes a device from the allowlist. Returns `true` if a
    /// device was actually removed.
    pub fn revoke(&self, id: &EndpointId) -> anyhow::Result<bool> {
        let mut file = self.load()?;
        let id_str = id.to_string();
        let before = file.devices.len();
        file.devices.retain(|d| d.id != id_str);
        let removed = file.devices.len() != before;
        if removed {
            self.save(&file)?;
        }
        Ok(removed)
    }

    /// Updates `last_seen` for `id` to now. No-op if the device
    /// isn't on the allowlist.
    pub fn touch_last_seen(&self, id: &EndpointId) -> anyhow::Result<()> {
        let mut file = self.load()?;
        let id_str = id.to_string();
        let mut changed = false;
        for device in &mut file.devices {
            if device.id == id_str {
                device.last_seen = Some(now_unix());
                changed = true;
            }
        }
        if changed {
            self.save(&file)?;
        }
        Ok(())
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Resolves the devices.json path for a data directory.
pub fn devices_path(data_dir: &Path) -> PathBuf {
    data_dir.join("devices.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn fresh_id() -> EndpointId {
        SecretKey::generate().public()
    }

    #[test]
    fn add_then_is_paired_and_list() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DeviceStore::new(tmp.path().join("devices.json"));
        let id = fresh_id();

        assert!(!store.is_paired(&id).expect("is_paired"));

        let device = store.add(&id, "My Phone".to_string()).expect("add");
        assert_eq!(device.name, "My Phone");
        assert!(store.is_paired(&id).expect("is_paired"));

        let list = store.list().expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id.to_string());
    }

    #[test]
    fn re_adding_replaces_the_existing_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DeviceStore::new(tmp.path().join("devices.json"));
        let id = fresh_id();

        store.add(&id, "Old Name".to_string()).expect("add");
        store.add(&id, "New Name".to_string()).expect("re-add");

        let list = store.list().expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "New Name");
    }

    #[test]
    fn revoke_removes_device() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DeviceStore::new(tmp.path().join("devices.json"));
        let id = fresh_id();

        store.add(&id, "Phone".to_string()).expect("add");
        assert!(store.revoke(&id).expect("revoke"));
        assert!(!store.is_paired(&id).expect("is_paired"));
        assert!(!store.revoke(&id).expect("revoke again"));
    }

    #[test]
    fn touch_last_seen_updates_only_matching_device() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DeviceStore::new(tmp.path().join("devices.json"));
        let id = fresh_id();
        let other = fresh_id();

        store.add(&id, "Phone".to_string()).expect("add");
        store.add(&other, "Other".to_string()).expect("add other");

        store.touch_last_seen(&id).expect("touch");

        let list = store.list().expect("list");
        let this = list.iter().find(|d| d.id == id.to_string()).unwrap();
        let that = list.iter().find(|d| d.id == other.to_string()).unwrap();
        assert!(this.last_seen.is_some());
        assert!(that.last_seen.is_none());
    }

    #[test]
    fn missing_file_is_treated_as_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DeviceStore::new(tmp.path().join("devices.json"));
        assert_eq!(store.list().expect("list"), vec![]);
    }
}
