//! `oino-settings`: pi 1.4.0 settings for Oino — global/project layering, migrations, typed
//! getters, locked atomic writes, and the filesystem layout of the agent and project dirs.
//!
//! Boundary: synchronous std I/O only (no async runtime); `oino-core` owns one
//! [`SettingsManager`] per cwd and hands [`Settings`] snapshots to blocking tasks.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`manager`] | `SettingsManager`, `FileSettingsStorage` | [`SettingsManager`]: create / in-memory, trust gating, reload, scoped writes, typed setters |
//! | [`view`] | `SettingsManager` getters | [`Settings`]: typed getters with pi defaults, layer access, extension settings layering |
//! | [`types`] | `settings-schema.ts`, `SETTINGS_DEFAULTS` | [`QueueMode`], [`CacheWarmingMode`], [`ProjectTrustDefault`], [`UiDensity`], resolved setting structs, [`PackageSource`], [`ResourceKind`], [`defaults`] |
//! | [`error`] | `SettingsError`, `collectSettingsDiagnostics` | [`SettingsScope`], [`SettingsError`], [`SettingsErrorKind`], [`InvalidSettingError`] |
//! | [`json`] | `deepMergeSettings`, `migrateSettings` | merge/migrate/path helpers over [`JsonObject`] |
//! | [`tools`] | `DEFAULT_TOOL_NAMES`, tool modifiers | [`tools::resolve_default_tools`], [`tools::apply_tool_modifiers`], [`tools::tool_list_error`] |
//! | [`paths`] | `config.ts`, `utils/paths.ts` | [`paths::agent_dir`], [`paths::project_dir`], file-name constants, [`paths::env`] names, tilde/`file://` resolution |
//! | [`storage`] | `proper-lockfile` use | [`storage::FileLock`] (`<file>.lock`), [`storage::write_atomic`] (shared with other agent-dir files) |
//!
//! # Semantics (pi parity)
//!
//! - Project settings (`<cwd>/.oino/settings.json`) load only when the project is trusted and
//!   override global ones (`<agent>/settings.json`); nested objects deep-merge, arrays replace.
//! - Resource arrays are read per layer with [`Settings::resource_entries`]; resolution
//!   (in `oino-resources`) combines both layers, project entries first.
//! - `httpProxy`, `cacheWarming`, `defaultProjectTrust` and `deviceId` read the global layer only.
//! - Unknown keys are preserved on write; legacy keys are migrated on load and on write.
//!
//! Oino keys: `extensionSettings`, `disabledExtensions`, `serve.*`, `ui.density`.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod error;
pub mod json;
pub mod manager;
pub mod paths;
pub mod storage;
pub mod tools;
pub mod types;
pub mod view;

pub use error::{InvalidSettingError, SettingsError, SettingsErrorKind, SettingsScope};
pub use json::JsonObject;
pub use manager::SettingsManager;
pub use types::{
    BranchSummarySettings, CacheWarmingMode, CompactionSettings, FilteredPackageSource,
    ImageSettings, PackageSource, ProjectTrustDefault, ProviderRetrySettings, QueueMode,
    ResourceKind, RetrySettings, ServeSettings, UiDensity, defaults,
};
pub use view::{CompactionTokenField, Settings};
