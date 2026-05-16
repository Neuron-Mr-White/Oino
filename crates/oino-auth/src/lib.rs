#![doc = "Credential storage and resolution for Oino providers."]
#![forbid(unsafe_code)]

pub const OPENROUTER_PROVIDER_ID: &str = "openrouter";
pub const OPENROUTER_ENV_VAR: &str = "OPENROUTER_API_KEY";

#[must_use]
pub fn crate_ready() -> bool { true }
