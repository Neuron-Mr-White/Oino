//! Protocol version constant and strict semver validation.

use serde::{Deserialize, Serialize};

/// The wire protocol version implemented by this crate.
pub const PROTO_VERSION: &str = "2.0.0";

/// Returns true if `s` is a strict semver string: `MAJOR.MINOR.PATCH`
/// with optional `-prerelease` and `+build` metadata (SemVer 2.0.0),
/// mirroring the TypeScript regex in `packages/protocol/src/version.ts`.
pub fn is_strict_semver(s: &str) -> bool {
    let (core, _build) = match s.split_once('+') {
        Some((core, build)) => (core, Some(build)),
        None => (s, None),
    };

    if let Some((_, build)) = s.split_once('+') {
        if build.is_empty() || !is_dot_separated_ident(build) {
            return false;
        }
    }

    let (version, prerelease) = match core.split_once('-') {
        Some((version, prerelease)) => (version, Some(prerelease)),
        None => (core, None),
    };

    if let Some(prerelease) = prerelease {
        if prerelease.is_empty() || !is_dot_separated_ident(prerelease) {
            return false;
        }
    }

    let mut parts = version.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };

    [major, minor, patch]
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn is_dot_separated_ident(s: &str) -> bool {
    s.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A string guaranteed (by construction via `TryFrom`/deserialize) to
/// be a strict semver value. Deserializes with validation so invalid
/// strings become `E_SCHEMA` at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Semver(String);

impl Semver {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Semver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Semver {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if is_strict_semver(&value) {
            Ok(Semver(value))
        } else {
            Err(format!(
                "must be a strict semver string (e.g. 1.0.0), got: {value}"
            ))
        }
    }
}

impl<'de> Deserialize<'de> for Semver {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Semver::try_from(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_basic_semver() {
        assert!(is_strict_semver("1.0.0"));
        assert!(is_strict_semver("0.0.1"));
        assert!(is_strict_semver("10.20.30"));
    }

    #[test]
    fn accepts_prerelease_and_build() {
        assert!(is_strict_semver("1.0.0-beta.1"));
        assert!(is_strict_semver("1.0.0+build.5"));
        assert!(is_strict_semver("2.0.0-beta.1+build.5"));
    }

    #[test]
    fn rejects_malformed() {
        assert!(!is_strict_semver("1.0"));
        assert!(!is_strict_semver("1"));
        assert!(!is_strict_semver("v1.0.0"));
        assert!(!is_strict_semver("1.0.0."));
        assert!(!is_strict_semver(""));
        assert!(!is_strict_semver("1.0.0-"));
        assert!(!is_strict_semver("1.0.0+"));
    }

    #[test]
    fn deserializes_valid_and_rejects_invalid() {
        let ok: Result<Semver, _> = serde_json::from_str("\"1.0.0\"");
        assert!(ok.is_ok());

        let bad: Result<Semver, _> = serde_json::from_str("\"1.0\"");
        assert!(bad.is_err());
    }
}
