//! Compaction settings (pi `CompactionSettings`, `DEFAULT_COMPACTION_SETTINGS`).

use serde::{Deserialize, Serialize};

/// pi `CompactionSettings` (`settings.compaction`, per-model overrides resolved by the caller).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSettings {
    /// Automatic compaction enabled.
    pub enabled: bool,
    /// Tokens kept free for the response; trigger is `tokens > window − reserve`.
    pub reserve_tokens: u64,
    /// Approximate tokens of recent history kept verbatim.
    pub keep_recent_tokens: u64,
}

/// pi `DEFAULT_COMPACTION_SETTINGS`.
pub const DEFAULT_COMPACTION_SETTINGS: CompactionSettings = CompactionSettings {
    enabled: true,
    reserve_tokens: 16384,
    keep_recent_tokens: 20000,
};

impl Default for CompactionSettings {
    fn default() -> Self {
        DEFAULT_COMPACTION_SETTINGS
    }
}
