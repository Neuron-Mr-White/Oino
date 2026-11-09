//! Pairing codes: generation, persistence (`pairing.json`) and
//! constant-time, single-use, expiring verification.
//!
//! One pairing attempt is in flight at a time (starting a new one
//! with `unipi-host pair` replaces any previous attempt). A code is
//! valid for 10 minutes, accepts at most 5 failed guesses before it
//! locks out (and must be regenerated), and is consumed (single-use)
//! on the first successful match.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// Pairing codes are valid for 10 minutes.
pub const PAIRING_TTL_SECS: u64 = 10 * 60;
/// After this many failed attempts, the pairing attempt locks out
/// (and must be regenerated with a fresh `unipi-host pair`).
pub const MAX_ATTEMPTS: u32 = 5;
/// Crockford base32 code length.
const CODE_LEN: usize = 10;

const CROCKFORD_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generates a fresh 10-character Crockford base32 code from a CSPRNG.
pub fn generate_code() -> String {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(CODE_LEN);
    for _ in 0..CODE_LEN {
        let idx = (rng.next_u32() as usize) % CROCKFORD_ALPHABET.len();
        out.push(CROCKFORD_ALPHABET[idx] as char);
    }
    out
}

/// A single in-flight pairing attempt, persisted to `pairing.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingState {
    pub code: String,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub used: bool,
}

impl PairingState {
    /// Creates a new pairing attempt with a freshly generated code,
    /// valid for [`PAIRING_TTL_SECS`] from now.
    pub fn new(now: u64) -> Self {
        PairingState {
            code: generate_code(),
            created_at: now,
            expires_at: now + PAIRING_TTL_SECS,
            attempts: 0,
            used: false,
        }
    }

    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    pub fn is_locked_out(&self) -> bool {
        self.attempts >= MAX_ATTEMPTS
    }

    /// Whether this attempt can still be used to pair: not expired,
    /// not already used, and not locked out.
    pub fn is_available(&self, now: u64) -> bool {
        !self.used && !self.is_expired(now) && !self.is_locked_out()
    }
}

/// Outcome of verifying a candidate code against the current
/// pairing attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The code matched an available (unexpired, unused, not
    /// locked-out) attempt. The attempt is now marked used.
    Accepted,
    /// No pairing attempt is in flight.
    NoPendingAttempt,
    /// The attempt that was in flight has expired.
    Expired,
    /// The attempt that was in flight was already consumed.
    AlreadyUsed,
    /// The attempt has seen too many failed guesses and is locked
    /// out.
    LockedOut,
    /// A pairing attempt is in flight and still available, but the
    /// supplied code didn't match (an attempt was recorded).
    Mismatch,
}

/// File-backed pairing store: one in-flight attempt, or none.
#[derive(Debug, Clone)]
pub struct PairingStore {
    path: PathBuf,
}

impl PairingStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        PairingStore { path: path.into() }
    }

    /// Starts a new pairing attempt, overwriting any previous one.
    /// Returns the generated code.
    pub fn start(&self, now: u64) -> anyhow::Result<PairingState> {
        let state = PairingState::new(now);
        crate::data_dir::write_json(&self.path, &state)?;
        Ok(state)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        crate::data_dir::remove_optional(&self.path)?;
        Ok(())
    }

    /// Verifies `candidate` against the current pairing attempt
    /// (constant-time comparison), updating `attempts`/`used` on
    /// disk as a side effect.
    pub fn verify(&self, candidate: &str, now: u64) -> anyhow::Result<VerifyOutcome> {
        let Some(mut state) = crate::data_dir::read_json::<PairingState>(&self.path)? else {
            return Ok(VerifyOutcome::NoPendingAttempt);
        };

        if state.used {
            return Ok(VerifyOutcome::AlreadyUsed);
        }
        if state.is_expired(now) {
            return Ok(VerifyOutcome::Expired);
        }
        if state.is_locked_out() {
            return Ok(VerifyOutcome::LockedOut);
        }

        let matches = constant_time_eq_str(&state.code, candidate);
        if matches {
            state.used = true;
            crate::data_dir::write_json(&self.path, &state)?;
            Ok(VerifyOutcome::Accepted)
        } else {
            state.attempts += 1;
            crate::data_dir::write_json(&self.path, &state)?;
            if state.is_locked_out() {
                Ok(VerifyOutcome::LockedOut)
            } else {
                Ok(VerifyOutcome::Mismatch)
            }
        }
    }

    /// Returns the current attempt, if any (for `unipi-host pair`
    /// to redisplay / inspect without mutating state).
    pub fn current(&self) -> anyhow::Result<Option<PairingState>> {
        crate::data_dir::read_json(&self.path)
    }
}

/// Constant-time string comparison. Codes are ASCII and fixed
/// length, so a length mismatch is safe to reveal early (it's not
/// secret information - only the code's identity is).
fn constant_time_eq_str(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

pub fn pairing_path(data_dir: &Path) -> PathBuf {
    data_dir.join("pairing.json")
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_has_expected_length_and_alphabet() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| CROCKFORD_ALPHABET.contains(&b)));
    }

    #[test]
    fn start_then_verify_correct_code_accepts_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let now = 1_000;
        let state = store.start(now).expect("start");

        let outcome = store.verify(&state.code, now + 1).expect("verify");
        assert_eq!(outcome, VerifyOutcome::Accepted);

        // Single-use: the same code doesn't work twice.
        let outcome2 = store.verify(&state.code, now + 1).expect("verify again");
        assert_eq!(outcome2, VerifyOutcome::AlreadyUsed);
    }

    #[test]
    fn wrong_code_is_mismatch_and_increments_attempts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let now = 1_000;
        store.start(now).expect("start");

        let outcome = store.verify("WRONGCODE1", now).expect("verify");
        assert_eq!(outcome, VerifyOutcome::Mismatch);

        let state = store.current().expect("current").expect("some");
        assert_eq!(state.attempts, 1);
    }

    #[test]
    fn five_failed_attempts_locks_out() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let now = 1_000;
        let state = store.start(now).expect("start");
        let wrong = if state.code == "0000000000" {
            "1111111111"
        } else {
            "0000000000"
        };

        for i in 0..MAX_ATTEMPTS {
            let outcome = store.verify(wrong, now).expect("verify");
            if i + 1 < MAX_ATTEMPTS {
                assert_eq!(outcome, VerifyOutcome::Mismatch, "attempt {i}");
            } else {
                assert_eq!(outcome, VerifyOutcome::LockedOut, "attempt {i}");
            }
        }

        // Even the correct code no longer works once locked out.
        let outcome = store
            .verify(&state.code, now)
            .expect("verify after lockout");
        assert_eq!(outcome, VerifyOutcome::LockedOut);
    }

    #[test]
    fn expired_code_is_rejected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let now = 1_000;
        let state = store.start(now).expect("start");

        let after_ttl = now + PAIRING_TTL_SECS;
        let outcome = store.verify(&state.code, after_ttl).expect("verify");
        assert_eq!(outcome, VerifyOutcome::Expired);
    }

    #[test]
    fn no_pending_attempt_when_nothing_started() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let outcome = store.verify("ANYTHING00", 0).expect("verify");
        assert_eq!(outcome, VerifyOutcome::NoPendingAttempt);
    }

    #[test]
    fn starting_a_new_attempt_replaces_the_old_one() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        let first = store.start(0).expect("start 1");
        let second = store.start(0).expect("start 2");
        assert_ne!(first.code, second.code);

        // The first code no longer verifies (it's gone).
        let outcome = store.verify(&first.code, 0).expect("verify old code");
        assert_eq!(outcome, VerifyOutcome::Mismatch);
    }

    #[test]
    fn clear_removes_the_pending_attempt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = PairingStore::new(tmp.path().join("pairing.json"));
        store.start(0).expect("start");
        store.clear().expect("clear");
        assert_eq!(store.current().expect("current"), None);
    }
}
