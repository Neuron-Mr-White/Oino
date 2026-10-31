//! `oino-resources`: resource loading for Oino — context files (`AGENTS.md` & co.),
//! SYSTEM/APPEND prompt sources, skills, prompt templates, theme files, the structured system
//! prompt, project trust and the keybindings file.
//!
//! Boundary: synchronous std I/O, no async runtime; `oino-core` calls it on blocking threads.
//! Packages and extensions are resolved by `oino-core` and contribute paths through
//! [`ResourceLoader::extend`].
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|---|
//! | [`loader`] | [`ResourceLoader`], [`ResourceLoaderOptions`], [`ResourceExtensionPaths`], [`ThemeResource`] |
//! | [`discovery`] | settings arrays + auto-discovery with `!glob`/`+path`/`-path`, precedence |
//! | [`context_files`] | [`ContextFile`], [`context_files::load_project_context_files`] |
//! | [`system_prompt`] | [`BuildSystemPromptOptions`], [`SystemPromptState`], [`system_prompt::build_system_prompt_sections`], [`system_prompt::build_system_prompt_state`], [`system_prompt::diff_system_prompt_sections`] |
//! | [`skills`] | [`Skill`], [`skills::load_skills`], [`skills::format_skills_for_prompt`], [`skills::expand_skill_command`] |
//! | [`templates`] | [`PromptTemplate`], [`templates::parse_command_args`], [`templates::substitute_args`], [`templates::expand_prompt_template`] |
//! | [`trust`] | [`trust::TrustStore`], [`trust::has_protected_resources`], [`trust::resolve_project_trust`] |
//! | [`keybindings`] | [`keybindings::KeybindingsFile`], [`keybindings::migrate_keybindings_config`], [`keybindings::is_valid_key_id`] |
//! | [`frontmatter`] | [`frontmatter::parse_frontmatter`] |
//! | [`diagnostics`], [`source`] | `diagnostics.ts`, `source-info.ts` | [`ResourceDiagnostic`], [`SourceInfo`], [`source::PathMetadata`] |
//!
//! Product substitutions (D13): the default preamble names Oino, the docs section points at the
//! bundled Oino docs (`docs_root`), and the bash env guideline is [`system_prompt::ENV_GUIDELINE`].

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

pub mod context_files;
pub mod diagnostics;
pub mod discovery;
pub mod frontmatter;
pub mod keybindings;
pub mod loader;
pub mod skills;
pub mod source;
pub mod system_prompt;
pub mod templates;
pub mod trust;
mod walk;

pub use context_files::ContextFile;
pub use diagnostics::{DiagnosticKind, ResourceDiagnostic};
pub use loader::{
    ExtraResourcePath, ResourceExtensionPaths, ResourceLoader, ResourceLoaderOptions, ThemeResource,
};
pub use skills::Skill;
pub use source::SourceInfo;
pub use system_prompt::{BuildSystemPromptOptions, SystemPromptState};
pub use templates::PromptTemplate;
