//! Typed setting values and pi `SETTINGS_DEFAULTS`.

use serde::{Deserialize, Serialize};

/// pi `SETTINGS_DEFAULTS` (runtime keys) plus the Oino keys of runtime §10.2.
pub mod defaults {
    /// `compaction.enabled`.
    pub const COMPACTION_ENABLED: bool = true;
    /// `compaction.reserveTokens`.
    pub const COMPACTION_RESERVE_TOKENS: u64 = 16_384;
    /// `compaction.keepRecentTokens`.
    pub const COMPACTION_KEEP_RECENT_TOKENS: u64 = 20_000;
    /// `branchSummary.reserveTokens`.
    pub const BRANCH_SUMMARY_RESERVE_TOKENS: u64 = 16_384;
    /// `branchSummary.skipPrompt`.
    pub const BRANCH_SUMMARY_SKIP_PROMPT: bool = false;
    /// `retry.enabled`.
    pub const RETRY_ENABLED: bool = true;
    /// `retry.maxRetries`.
    pub const RETRY_MAX_RETRIES: u64 = 3;
    /// `retry.baseDelayMs`.
    pub const RETRY_BASE_DELAY_MS: u64 = 2_000;
    /// `retry.maxAgentDelayMs` (pi-ai `DEFAULT_MAX_AGENT_RETRY_DELAY_MS`).
    pub const RETRY_MAX_AGENT_DELAY_MS: u64 = 60_000;
    /// `retry.provider.maxRetryDelayMs`.
    pub const RETRY_PROVIDER_MAX_RETRY_DELAY_MS: u64 = 60_000;
    /// `httpIdleTimeoutMs` (pi `DEFAULT_HTTP_IDLE_TIMEOUT_MS`).
    pub const HTTP_IDLE_TIMEOUT_MS: u64 = 300_000;
    /// `showCacheMissNotices`.
    pub const SHOW_CACHE_MISS_NOTICES: bool = false;
    /// `enableSkillCommands`.
    pub const ENABLE_SKILL_COMMANDS: bool = true;
    /// `images.autoResize`.
    pub const IMAGES_AUTO_RESIZE: bool = true;
    /// `images.blockImages`.
    pub const IMAGES_BLOCK_IMAGES: bool = false;
    /// Runtime default thinking level when `defaultThinkingLevel` is unset (pi `defaults.ts`).
    pub const THINKING_LEVEL: oino_ai::ThinkingLevel = oino_ai::ThinkingLevel::Medium;
    /// `serve.bind`.
    pub const SERVE_BIND: &str = "127.0.0.1:7420";
    /// `serve.tls`.
    pub const SERVE_TLS: bool = true;
    /// `serve.allowRemote`.
    pub const SERVE_ALLOW_REMOTE: bool = false;
}

/// Steering / follow-up queue delivery (`steeringMode`, `followUpMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueMode {
    /// Deliver every queued message at once.
    #[serde(rename = "all")]
    All,
    /// Deliver one queued message per turn (pi default).
    #[default]
    #[serde(rename = "one-at-a-time")]
    OneAtATime,
}

/// `cacheWarming` profile (global only, pi `CacheWarmingMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheWarmingMode {
    /// Never warm.
    Off,
    /// Warm while streaming (pi default).
    #[default]
    Streaming,
    /// Also warm between agent runs.
    Idle,
}

/// `defaultProjectTrust` (global only, pi `DefaultProjectTrust`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectTrustDefault {
    /// Ask interactively; non-interactive modes treat it as untrusted (pi default).
    #[default]
    Ask,
    /// Trust every project.
    Always,
    /// Trust no project.
    Never,
}

/// `ui.density`: transcript density for render specs (Oino key).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiDensity {
    /// Minimal transcript.
    Simple,
    /// Default transcript.
    #[default]
    Regular,
    /// Everything visible.
    Advanced,
}

/// Resolved compaction settings (pi `getCompactionSettings`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSettings {
    /// Auto-compaction on/off.
    pub enabled: bool,
    /// Tokens reserved for the prompt and response.
    pub reserve_tokens: u64,
    /// Recent tokens kept verbatim.
    pub keep_recent_tokens: u64,
}

/// Resolved branch summary settings (pi `getBranchSummarySettings`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummarySettings {
    /// Tokens reserved for the prompt and response.
    pub reserve_tokens: u64,
    /// Skip the "Summarize branch?" prompt.
    pub skip_prompt: bool,
}

/// Resolved agent-level retry settings (pi `getRetrySettings`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrySettings {
    /// Auto-retry on/off.
    pub enabled: bool,
    /// Maximum attempts.
    pub max_retries: u64,
    /// Exponential backoff base delay.
    pub base_delay_ms: u64,
    /// Upper bound for one agent-level delay.
    pub max_agent_delay_ms: u64,
}

/// Resolved provider/SDK retry settings (pi `getProviderRetrySettings`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRetrySettings {
    /// Request timeout, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Provider retry attempts, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u64>,
    /// Maximum server-requested delay before failing.
    pub max_retry_delay_ms: u64,
}

/// Resolved image settings (`images.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    /// Resize images to 2000x2000 maximum.
    pub auto_resize: bool,
    /// Never send images to providers.
    pub block_images: bool,
}

/// Resolved `serve.*` settings (Oino key, modes §5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServeSettings {
    /// Listen address.
    pub bind: String,
    /// Serve TLS.
    pub tls: bool,
    /// Accept non-loopback clients.
    pub allow_remote: bool,
}

/// A `packages[]` entry (pi `PackageSource`): a source string or a filtered object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageSource {
    /// Load every resource of the package.
    Source(String),
    /// Load a filtered subset.
    Filtered(FilteredPackageSource),
}

impl PackageSource {
    /// The package source string.
    #[must_use]
    pub fn source(&self) -> &str {
        match self {
            Self::Source(source) => source,
            Self::Filtered(filtered) => &filtered.source,
        }
    }
}

/// Object form of [`PackageSource`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilteredPackageSource {
    /// Package source.
    pub source: String,
    /// When `false`, start empty and apply only explicit patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoload: Option<bool>,
    /// Extension patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    /// Skill patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Prompt template patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Vec<String>>,
    /// Theme patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<String>>,
}

/// Resource arrays of settings (`extensions`, `skills`, `prompts`, `themes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    /// `extensions`.
    Extensions,
    /// `skills`.
    Skills,
    /// `prompts`.
    Prompts,
    /// `themes`.
    Themes,
}

impl ResourceKind {
    /// Every kind in pi `RESOURCE_TYPES` order.
    pub const ALL: [Self; 4] = [Self::Extensions, Self::Skills, Self::Prompts, Self::Themes];

    /// The settings key, which is also the conventional directory name.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Extensions => "extensions",
            Self::Skills => "skills",
            Self::Prompts => "prompts",
            Self::Themes => "themes",
        }
    }
}

/// Parse a JSON string value into one of the enums above (`None` for anything else).
pub(crate) fn parse_enum<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Option<T> {
    value
        .is_string()
        .then(|| serde_json::from_value(value.clone()).ok())
        .flatten()
}
