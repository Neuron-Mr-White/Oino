//! `oino-settings`: settings for Oino — global/project layering, migrations, typed
//! getters, locked atomic writes, and the filesystem layout of the agent and project dirs.
//!
//! Boundary: synchronous std I/O only (no async runtime); `oino-core` owns one
//! [`SettingsManager`] per cwd and hands [`Settings`] snapshots to blocking tasks.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`manager`] | [`SettingsManager`]: create / in-memory, trust gating, reload, scoped writes, typed setters |
//! | [`view`] | [`Settings`]: typed getters with defaults, layer access, extension settings layering |
//! | [`types`] | [`QueueMode`], [`CacheWarmingMode`], [`ProjectTrustDefault`], [`UiDensity`], resolved setting structs, [`PackageSource`], [`ResourceKind`], [`defaults`] |
//! | [`error`] | [`SettingsScope`], [`SettingsError`], [`SettingsErrorKind`], [`InvalidSettingError`] |
//! | [`json`] | merge/migrate/path helpers over [`JsonObject`] |
//! | [`tools`] | [`tools::resolve_default_tools`], [`tools::apply_tool_modifiers`], [`tools::tool_list_error`] |
//! | [`paths`] | [`paths::agent_dir`], [`paths::project_dir`], file-name constants, [`paths::env`] names, tilde/`file://` resolution |
//! | [`storage`] | [`storage::FileLock`] (`<file>.lock`), [`storage::write_atomic`] (shared with other agent-dir files) |
//! | [`secrets`] | [`SecretStore`]: `<agent>/secrets.json` (0600, locked), extension secrets and secret settings fields |
//!
//! # Semantics
//!
//! - Project settings (`<cwd>/.oino/settings.json`) load only when the project is trusted and
//!   override global ones (`<agent>/settings.json`); nested objects deep-merge, arrays replace.
//! - Resource arrays are read per layer with [`Settings::resource_entries`]; resolution
//!   (in `oino-resources`) combines both layers, project entries first.
//! - `httpProxy`, `cacheWarming`, `defaultProjectTrust` and `deviceId` read the global layer only.
//! - Unknown keys are preserved on write; legacy keys are migrated on load and on write.
//!
//! Oino keys: `extensionSettings`, `disabledExtensions`, `serve.*`, `ui.density`.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]
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
pub mod secrets;
pub mod storage;
pub mod tools;
pub mod types;
pub mod view;

pub use error::{InvalidSettingError, SettingsError, SettingsErrorKind, SettingsScope};
pub use json::JsonObject;
pub use manager::SettingsManager;
pub use secrets::SecretStore;
pub use types::{
    BranchSummarySettings, CacheWarmingMode, CompactionSettings, FilteredPackageSource,
    ImageSettings, PackageSource, ProjectTrustDefault, ProviderRetrySettings, QueueMode,
    ResourceKind, RetrySettings, ServeSettings, UiDensity, defaults,
};
pub use view::{CompactionTokenField, Settings};
