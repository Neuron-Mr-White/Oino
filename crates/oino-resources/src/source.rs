//! Resource provenance (pi `core/source-info.ts`, `PathMetadata`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a resource was configured (pi `SourceScope`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceScope {
    /// Agent dir / user settings.
    User,
    /// Project `.oino` / project settings / project `.agents`.
    Project,
    /// CLI flags or other session-only sources.
    #[default]
    Temporary,
}

/// Whether a resource came from a package or a top-level path (pi `SourceOrigin`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceOrigin {
    /// Provided by an installed package.
    Package,
    /// A path from settings, auto-discovery or the CLI.
    #[default]
    TopLevel,
}

/// Resolution metadata of a discovered path (pi `PathMetadata`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMetadata {
    /// `auto`, `local`, `cli`, a package source, … (pi keeps this a free string).
    pub source: String,
    /// Scope.
    pub scope: SourceScope,
    /// Origin.
    pub origin: SourceOrigin,
    /// Directory relative entries resolve against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<PathBuf>,
}

/// Provenance attached to a loaded resource (pi `SourceInfo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    /// Resource path.
    pub path: PathBuf,
    /// Source kind (see [`PathMetadata::source`]).
    pub source: String,
    /// Scope.
    pub scope: SourceScope,
    /// Origin.
    pub origin: SourceOrigin,
    /// Base dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<PathBuf>,
}

impl SourceInfo {
    /// pi `createSourceInfo`.
    #[must_use]
    pub fn from_metadata(path: &Path, metadata: &PathMetadata) -> Self {
        Self {
            path: path.to_path_buf(),
            source: metadata.source.clone(),
            scope: metadata.scope,
            origin: metadata.origin,
            base_dir: metadata.base_dir.clone(),
        }
    }

    /// pi `createSyntheticSourceInfo` (scope defaults to temporary, origin to top-level).
    #[must_use]
    pub fn synthetic(
        path: &Path,
        source: &str,
        scope: Option<SourceScope>,
        base_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            source: source.to_owned(),
            scope: scope.unwrap_or_default(),
            origin: SourceOrigin::TopLevel,
            base_dir,
        }
    }
}
