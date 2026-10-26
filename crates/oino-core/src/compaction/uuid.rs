//! `UUIDv7` routing ids for summary requests (pi-ai `uuidv7`), without an RNG dependency:
//! randomness comes from std's per-process random hasher keys.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_u64() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// A fresh RFC 9562 version-7 UUID string.
pub(crate) fn uuidv7() -> String {
    let millis = u64::try_from(oino_agent::now_ms()).unwrap_or(0) & 0xffff_ffff_ffff;
    let random_a = random_u64();
    let random_b = random_u64();
    let high = (millis << 16) | 0x7000 | (random_a & 0x0fff);
    let low = (random_b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        high >> 32,
        (high >> 16) & 0xffff,
        high & 0xffff,
        low >> 48,
        low & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_and_version() {
        let id = uuidv7();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "7");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(id, uuidv7());
    }
}
