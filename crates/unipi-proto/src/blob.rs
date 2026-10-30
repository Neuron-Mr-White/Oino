//! Blob channel wire types (DESIGN.md §3.9-adjacent "media + attachments",
//! UNI-157). Mirrors `packages/protocol/src/control.ts`.
//!
//! Two requests, same shape as `board_http` (phone⇄host stream per
//! request, see `crate::board` doc comment):
//!
//! `blob_put` stream:
//! - client → `hello{req:{op:"blob_put", sha256, size, mime, name?, offset?}}`,
//!   then exactly `size - offset.unwrap_or(0)` bytes in kind-2 frames.
//! - host → `ok{data:{have:true}}` immediately (already has the sha256; the
//!   phone must not send any bytes), or `ok{data:{have:false}}` followed by
//!   reading the body, verifying sha256+size, then `exit{code:0}`. A size
//!   over the cap or a sha256 mismatch is an `err` envelope.
//! - Resumable: a client that knows the host already has a *partial* blob
//!   (from a previous `blob_put` that was interrupted) retries with `offset`
//!   set to how many bytes it already sent; the host appends. The host never
//!   reports how much of a partial blob it holds — the phone must track its
//!   own upload progress and retry from 0 on any doubt.
//!
//! `blob_get` stream:
//! - client → `hello{req:{op:"blob_get", sha256 | path, range?}}`.
//! - host → `ok{data:{size, mime}}`, then the bytes in kind-1 frames, then
//!   `exit{code:0}`. `path` is only allowed for files under an allowed root
//!   (the blobs dir, or a path the pi bridge registered as an attachment);
//!   anything else is `err{E_UNAUTHORIZED}`.

use serde::{Deserialize, Serialize};

/// Largest blob the host accepts. UNI-278: was 50 MiB, which made Files →
/// Upload and `unipi-host push` refuse any big file; uploads are streamed to
/// disk in 256 KiB frames, so size only costs disk.
pub const BLOB_MAX_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// A lowercase hex sha256 digest: exactly 64 hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Sha256Hex {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            Ok(Sha256Hex(value))
        } else {
            Err("sha256 must be 64 lowercase hex characters".to_string())
        }
    }
}

impl<'de> Deserialize<'de> for Sha256Hex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Sha256Hex::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A blob size in bytes: 1..=`BLOB_MAX_SIZE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BlobSize(u64);

impl BlobSize {
    pub fn value(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for BlobSize {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if (1..=BLOB_MAX_SIZE).contains(&value) {
            Ok(BlobSize(value))
        } else {
            Err(format!(
                "size must be 1..={BLOB_MAX_SIZE} bytes, got {value}"
            ))
        }
    }
}

impl<'de> Deserialize<'de> for BlobSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u64::deserialize(deserializer)?;
        BlobSize::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A `blob_put`/`blob_get` MIME type: 1..=256 printable ASCII characters
/// (same shape as `board::ContentType`, named separately so each can grow
/// its own rules later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BlobMime(String);

impl BlobMime {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BlobMime {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 256 {
            return Err(format!(
                "mime must be 1..=256 characters, got {}",
                value.len()
            ));
        }
        if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Err("mime must be printable ASCII".into());
        }
        Ok(BlobMime(value))
    }
}

impl<'de> Deserialize<'de> for BlobMime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BlobMime::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A blob display name: at most 256 characters, no path separators or
/// control characters (it is never used as a filesystem path, only shown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BlobName(String);

impl BlobName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BlobName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if len == 0 || len > 256 {
            return Err(format!("name must be 1..=256 characters, got {len}"));
        }
        if value
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        {
            return Err("name must not contain control characters or path separators".into());
        }
        Ok(BlobName(value))
    }
}

impl<'de> Deserialize<'de> for BlobName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BlobName::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// An absolute filesystem path for `blob_get{path}`: non-empty, no NUL
/// bytes. Allow-listing (blobs dir, or a bridge-registered attachment) is
/// the host's job, not this type's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BlobPath(String);

impl BlobPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BlobPath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 4096 {
            return Err(format!(
                "path must be 1..=4096 characters, got {}",
                value.len()
            ));
        }
        if value.bytes().any(|b| b == 0) {
            return Err("path must not contain NUL bytes".into());
        }
        Ok(BlobPath(value))
    }
}

impl<'de> Deserialize<'de> for BlobPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BlobPath::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A byte range for `blob_get{range}`: `start..=end` inclusive, both in
/// 0..size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRange {
    pub start: u64,
    pub end: u64,
}

/// `ok.data` for a `blob_put` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobPutData {
    /// The host already has this exact sha256 (dedup): the phone sends no
    /// bytes and the stream ends after this reply (no `exit` needed; the
    /// host closes after sending `ok`).
    pub have: bool,
    /// The blob's absolute path on the host (`<blobs dir>/<sha256>`). The
    /// phone passes this straight to the pi bridge as a `prompt.images[].path`
    /// — the bridge and the host run on the same machine, so it reads the
    /// file directly with no extra `blob_get` round trip.
    pub path: String,
}

/// `ok.data` for a `blob_get` request: the blob's size and mime before the
/// bytes (kind-1 frames) follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobGetData {
    pub size: u64,
    pub mime: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_validates() {
        let good = "a".repeat(64);
        assert!(Sha256Hex::try_from(good).is_ok());
        assert!(Sha256Hex::try_from("A".repeat(64)).is_err());
        assert!(Sha256Hex::try_from("a".repeat(63)).is_err());
        assert!(Sha256Hex::try_from("g".repeat(64)).is_err());
    }

    #[test]
    fn blob_size_bounds() {
        assert!(BlobSize::try_from(0).is_err());
        assert!(BlobSize::try_from(1).is_ok());
        assert!(BlobSize::try_from(BLOB_MAX_SIZE).is_ok());
        assert!(BlobSize::try_from(BLOB_MAX_SIZE + 1).is_err());
    }

    #[test]
    fn blob_name_rejects_separators() {
        assert!(BlobName::try_from("photo.jpg".to_string()).is_ok());
        assert!(BlobName::try_from("a/b".to_string()).is_err());
        assert!(BlobName::try_from("a\\b".to_string()).is_err());
        assert!(BlobName::try_from(String::new()).is_err());
    }

    #[test]
    fn blob_mime_validates() {
        assert!(BlobMime::try_from("image/jpeg".to_string()).is_ok());
        assert!(BlobMime::try_from(String::new()).is_err());
        assert!(BlobMime::try_from("a\r\nb".to_string()).is_err());
    }
}
