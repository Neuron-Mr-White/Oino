//! `oino-models`: the pi 1.4.0 model registry, `models.json`, model resolution, `auth.json`
//! storage and virtual models for Oino.
//!
//! Providers are WASM components: this crate never streams. It stores provider
//! *registrations* (extension-api §4.4) and their models, merges `models.json` and config
//! overrides exactly in pi's order, resolves credentials per request (core passes them to the
//! component as `StreamOptions.apiKey`/`headers`) and resolves `--model`/`--models`/defaults.
//! It has no async runtime; OAuth refresh is an injected [`OAuthRefresher`] future.
//!
//! # API map
//!
//! | Item | pi equivalent | Purpose |
//! |---|---|---|
//! | [`ModelRegistry`] | `ModelRegistry` / `ModelRuntime` | shared catalog + auth (`Arc<RwLock>`) |
//! | [`ProviderRegistration`], [`ProviderConfigOverride`] | native provider, `registerProvider(name, config)` | provider data |
//! | [`ModelsJson`] | `ModelConfig` | `models.json` load with pi diagnostics |
//! | [`AuthStorage`], [`AuthCredential`] | `AuthStorage`, `Credential` | `auth.json` with locking, runtime keys |
//! | [`auth::oauth`] | `auth/resolve.ts` | OAuth expiry + locked refresh |
//! | [`config_value`] | `resolve-config-value.ts` | `literal \| $ENV \| ${ENV} \| !command` |
//! | [`resolver`] | `model-resolver.ts`, `cycleModel` | `--model`, scopes, initial model, restore, cycling |
//! | [`virtual_models`] | `virtual-models.ts` | definitions, route wire types, validation, branch state |
//! | [`auth_check`] | `cli/auth-check.ts` | `oino auth check` (exit 0/1/2) |
//! | [`ModelCache`] | `models-store.json` | `<agent>/cache/models/<provider>.json` |
//! | [`auth::guidance`] | `auth-guidance.ts` | "No API key found …" texts |
//!
//! # Deviations from pi
//!
//! - No built-in catalog: models come from registrations (static or `list-models`) and
//!   `models.json`. A model is *available* only when its provider has credentials **and** a loaded
//!   component implements its API ([`ModelRegistry::diagnostics`] lists the others).
//! - Config overrides keep the provider component's registration (pi replaces a native provider).
//! - Registration `authHeader` is informational; `Authorization: Bearer` is injected only for
//!   `models.json`/override `authHeader: true`, like pi.
//! - Credential order follows pi's code: `--api-key` > `auth.json` > `models.json`/override
//!   `apiKey` > registration `authEnv` (runtime.md §13 lists env before `apiKey`; pi's docs and
//!   code put `apiKey` first).
//! - `models.json` `compat` is only checked for being an object; JSON syntax errors use
//!   `serde_json` wording; `oauth: "radius"` validates but has no flow in core.
//! - Virtual router state equal to the current state is not re-stored (no object identity
//!   across WASM).

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

pub mod auth;
pub mod auth_check;
mod cache;
mod collate;
mod compose;
pub mod config_value;
pub mod env;
mod glob;
mod json_text;
mod models_json;
mod registration;
mod registry;
pub mod resolver;
mod schema;
pub mod virtual_models;

pub use auth::credential::{
    ApiKeyCredential, AuthCredential, CredentialInfo, CredentialType, OAuthCredential,
};
pub use auth::oauth::{OAuthError, OAuthRefresher};
pub use auth::storage::{AuthStorage, AuthStorageError};
pub use cache::{CacheError, CachedModels, ModelCache};
pub use config_value::{ConfigValueError, resolve_config_value};
pub use env::{EnvSource, MapEnv, ProcessEnv};
pub use models_json::{
    ModelsJson, ModelsJsonModel, ModelsJsonModelOverride, ModelsJsonProvider, PartialCost,
    SamplingByLevel,
};
pub use registration::{
    AuthHeaderMode, ProviderConfigOverride, ProviderModelConfig, ProviderModels,
    ProviderRegistration,
};
pub use registry::{
    ApiKeyError, AuthCheck, AuthOverrides, AuthResolveError, AuthStatus, AuthStatusSource,
    ModelRegistry, ModelRegistryOptions, RegistryDiagnostic, RegistryError, RequestAuth,
    RequestAuthError, ResolvedAuth,
};
pub use virtual_models::{
    RouteReason, RouteRequest, RouteResult, VIRTUAL_MODEL_API, VirtualModelDefinition,
    VirtualModelRegistry,
};
