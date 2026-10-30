//! The host's iroh secret key: loaded from `secret.key` if present,
//! generated and persisted (mode `0600`) otherwise.

use std::path::Path;

use iroh::SecretKey;

use crate::data_dir::write_atomic;

/// Loads the secret key from `path`, generating and persisting a new
/// one if the file doesn't exist yet.
pub fn load_or_create(path: &Path) -> anyhow::Result<SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => parse_secret_key(&bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            write_atomic(path, &key.to_bytes())?;
            Ok(key)
        }
        Err(err) => Err(err.into()),
    }
}

fn parse_secret_key(bytes: &[u8]) -> anyhow::Result<SecretKey> {
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("secret.key must be exactly 32 bytes, got {}", bytes.len()))?;
    Ok(SecretKey::from_bytes(&array))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_persists_a_new_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.key");
        assert!(!path.exists());

        let key = load_or_create(&path).expect("load_or_create");
        assert!(path.exists());

        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes, key.to_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn persisted_key_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.key");
        load_or_create(&path).expect("load_or_create");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn loads_an_existing_key_unchanged() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.key");

        let first = load_or_create(&path).expect("first load");
        let second = load_or_create(&path).expect("second load");
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn rejects_a_malformed_key_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.key");
        std::fs::write(&path, b"too short").expect("write");
        assert!(load_or_create(&path).is_err());
    }
}
