//! Resource diagnostics (pi `core/diagnostics.ts`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Diagnostic severity / kind (pi `ResourceDiagnostic.type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticKind {
    /// Non-fatal problem; the resource may still load.
    Warning,
    /// The resource could not be loaded.
    Error,
    /// Two resources share a name; the first wins.
    Collision,
}

/// Resource type of a collision (pi `ResourceCollision.resourceType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollisionResource {
    /// Extension.
    Extension,
    /// Skill.
    Skill,
    /// Prompt template.
    Prompt,
    /// Theme.
    Theme,
}

/// Name collision details (pi `ResourceCollision`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCollision {
    /// Resource type.
    pub resource_type: CollisionResource,
    /// Colliding name.
    pub name: String,
    /// Path that was kept.
    pub winner_path: PathBuf,
    /// Path that was dropped.
    pub loser_path: PathBuf,
}

/// One resource diagnostic (pi `ResourceDiagnostic`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDiagnostic {
    /// Kind.
    #[serde(rename = "type")]
    pub kind: DiagnosticKind,
    /// pi message, e.g. `description is required`.
    pub message: String,
    /// Affected path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Collision details (kind = collision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collision: Option<ResourceCollision>,
}

impl ResourceDiagnostic {
    /// A warning for `path`.
    #[must_use]
    pub fn warning(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            kind: DiagnosticKind::Warning,
            message: message.into(),
            path: Some(path.into()),
            collision: None,
        }
    }

    /// An error for `path`.
    #[must_use]
    pub fn error(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            kind: DiagnosticKind::Error,
            message: message.into(),
            path: Some(path.into()),
            collision: None,
        }
    }

    /// A first-wins name collision (`path` = loser).
    #[must_use]
    pub fn collision(
        message: impl Into<String>,
        resource_type: CollisionResource,
        name: &str,
        winner: impl Into<PathBuf>,
        loser: impl Into<PathBuf>,
    ) -> Self {
        let loser = loser.into();
        Self {
            kind: DiagnosticKind::Collision,
            message: message.into(),
            path: Some(loser.clone()),
            collision: Some(ResourceCollision {
                resource_type,
                name: name.to_owned(),
                winner_path: winner.into(),
                loser_path: loser,
            }),
        }
    }
}
