#![doc = r#"Built-in Oino contribution catalogs backed by the extension registry model.

`oino-extension-builtins` is the adapter that presents Oino's built-in tools,
commands, keymaps, hooks, resources, settings pages, themes, and provider metadata as
regular extension-kernel registry contributions. This lets built-ins and external
extensions flow through the same policy, conflict, snapshot, and management surfaces.

## Boundary

This crate translates existing Oino surfaces into `oino-extension-core` contribution
registries. It does not execute tools, render UI, discover extension manifests, load
packages, persist extension state, mutate keymap/settings files, or decide registry
policy. `oino-tools`, `oino-tui`, `oino-resource`, and provider/app crates keep owning
the runtime behavior; this crate only mirrors their current metadata into built-in
registry entries with the built-in source and active lifecycle.

## Public API map

- [`BUILTIN_EXTENSION_ID`] is the synthetic owner for all built-in contributions.
- [`OPTIONAL_BUILTIN_PACKAGES_DIR`] and [`optional_builtin_packages`] expose checked-in,
  Oino-owned packages that users can install explicitly from `/extensions`.
- [`BuiltinRegistryCatalog`] groups the built-in [`oino_extension_core::ToolRegistry`],
  [`oino_extension_core::CommandRegistry`], [`oino_extension_core::KeymapRegistry`],
  [`oino_extension_core::HookRegistry`],
  [`oino_extension_core::SettingsPageRegistry`],
  [`oino_extension_core::ThemeRegistry`],
  [`oino_extension_core::ProviderModelRegistry`], and
  [`oino_extension_core::ResourceRegistry`]. [`BuiltinRegistryCatalog::from_parts`]
  builds the catalog from the current tool map, keymap config, resource catalog, and
  extension/runtime model ids.
- [`tool_registry_from_tools`] and [`tool_contribution_from_definition`] turn
  model-visible built-in tools into registry contributions while preserving sequential
  vs. parallel execution metadata.
- [`command_registry`], [`keymap_registry`], [`hook_registry`],
  [`settings_page_registry`], [`theme_registry`], [`provider_registry`], and
  [`resource_registry`] mirror the corresponding built-in Oino surfaces.
- [`BuiltinRegistryError`] keeps identifier and registry validation failures typed for
  the extension manager and tests.

## Contributor rules

Keep this crate a metadata mirror. When adding or renaming built-in commands, key actions,
settings pages, chat styles, provider metadata, tools, or resource kinds, update the
owning crate first and then update this catalog, extension docs, and registry tests so
`/extensions` shows the same surface the app actually exposes. Preserve deterministic ids
and slugs because user policy overrides can reference them. Do not bypass extension
policy by special-casing built-ins here; built-ins should remain registry entries whose
source and lifecycle make them enabled by default unless policy disables them.
"#]
#![forbid(unsafe_code)]

use oino_agent_loop::{Tool, ToolDefinition, ToolExecutionMode as AgentToolExecutionMode};
use oino_extension_core::{
    CommandContribution, CommandRegistry, ContributionId, ContributionMetadata, ExtensionCoreError,
    ExtensionId, HookContribution, HookEventKind, HookMode, HookRegistry, KeymapContribution,
    KeymapRegistry, LifecycleState, ProviderContribution, ProviderModelRegistry, RegistryEntryKey,
    RegistryValidationError, ResourceContribution, ResourceKind, ResourceRegistry,
    SettingsPageContribution, SettingsPageRegistry, SourceDescriptor, SourceKind, SourceScope,
    ThemeContribution, ThemeRegistry, ToolContribution, ToolExecutionMode, ToolRegistry,
};
use oino_resource::ResourceCatalog;
use oino_tui::{
    chat_style_value, key_action_rows, ChatStyle, KeymapConfig, SettingsMenuItem, COMMANDS,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

pub const BUILTIN_EXTENSION_ID: &str = "oino.builtins";
pub const OPTIONAL_BUILTIN_PACKAGES_DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../extensions/built-in");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionalBuiltinPackage {
    pub id: &'static str,
    pub display_name: &'static str,
    pub directory_name: &'static str,
    pub description: &'static str,
}

impl OptionalBuiltinPackage {
    #[must_use]
    pub fn path(self) -> PathBuf {
        Path::new(OPTIONAL_BUILTIN_PACKAGES_DIR).join(self.directory_name)
    }
}

pub const OPTIONAL_BUILTIN_PACKAGES: &[OptionalBuiltinPackage] = &[
    OptionalBuiltinPackage {
        id: "oino.footer_status",
        display_name: "Oino Footer Status",
        directory_name: "footer-status",
        description:
            "Composer-adjacent model, thinking, working-directory, and context status lines",
    },
    OptionalBuiltinPackage {
        id: "oino.ralph_loop",
        display_name: "Oino Ralph Loop",
        directory_name: "ralph-loop",
        description: "Oino-native iterative development loop state, commands, and promise tags",
    },
    OptionalBuiltinPackage {
        id: "oino.mode_sandbox",
        display_name: "Oino Mode Sandbox",
        directory_name: "mode-sandbox",
        description: "Read/plan/work sandbox profiles with global defaults and project overrides",
    },
    OptionalBuiltinPackage {
        id: "oino.notify",
        display_name: "Oino Notify",
        directory_name: "notify",
        description: "ntfy notifications for selected Oino lifecycle events",
    },
    OptionalBuiltinPackage {
        id: "oino.craft_skill",
        display_name: "Oino Craft Skill",
        directory_name: "craft-skill",
        description: "Oino-native skill for creating and validating reusable skills",
    },
    OptionalBuiltinPackage {
        id: "oino.vcc",
        display_name: "Oino VCC",
        directory_name: "vcc",
        description: "Deterministic Oino session compaction and recall commands/tools",
    },
    OptionalBuiltinPackage {
        id: "oino.ask_user",
        display_name: "Oino Ask User",
        directory_name: "ask-user",
        description: "Model-visible structured question tool backed by an Oino TUI modal",
    },
    OptionalBuiltinPackage {
        id: "oino.9router",
        display_name: "Oino 9router",
        directory_name: "9router",
        description: "9router auth/router integration with external endpoint setup and version fallback guidance",
    },
];

#[must_use]
pub fn optional_builtin_packages() -> &'static [OptionalBuiltinPackage] {
    OPTIONAL_BUILTIN_PACKAGES
}

pub fn optional_builtin_package_path(id: &str) -> Option<PathBuf> {
    optional_builtin_packages()
        .iter()
        .find(|package| package.id == id || package.directory_name == id)
        .copied()
        .map(OptionalBuiltinPackage::path)
}

#[derive(Debug, Error)]
pub enum BuiltinRegistryError {
    #[error(transparent)]
    Core(#[from] ExtensionCoreError),
    #[error(transparent)]
    Validation(#[from] RegistryValidationError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinRegistryCatalog {
    pub tools: ToolRegistry,
    pub commands: CommandRegistry,
    pub keymaps: KeymapRegistry,
    pub hooks: HookRegistry,
    pub settings_pages: SettingsPageRegistry,
    pub themes: ThemeRegistry,
    pub providers: ProviderModelRegistry,
    pub resources: ResourceRegistry,
}

impl BuiltinRegistryCatalog {
    pub fn from_parts(
        tools: &BTreeMap<String, Arc<dyn Tool>>,
        keymap: &KeymapConfig,
        resources: &ResourceCatalog,
        openrouter_models: impl IntoIterator<Item = String>,
    ) -> Result<Self, BuiltinRegistryError> {
        Ok(Self {
            tools: tool_registry_from_tools(tools)?,
            commands: command_registry()?,
            keymaps: keymap_registry(keymap)?,
            hooks: hook_registry()?,
            settings_pages: settings_page_registry()?,
            themes: theme_registry()?,
            providers: provider_registry(openrouter_models)?,
            resources: resource_registry(resources)?,
        })
    }

    #[must_use]
    pub fn total_contributions(&self) -> usize {
        self.tools.inner().len()
            + self.commands.inner().len()
            + self.keymaps.inner().len()
            + self.hooks.inner().len()
            + self.settings_pages.inner().len()
            + self.themes.inner().len()
            + self.providers.inner().len()
            + self.resources.inner().len()
    }
}

pub fn tool_registry_from_tools(
    tools: &BTreeMap<String, Arc<dyn Tool>>,
) -> Result<ToolRegistry, BuiltinRegistryError> {
    let mut registry = ToolRegistry::tools();
    for tool in tools.values() {
        let definition = tool.definition();
        let execution_mode = match tool.execution_mode() {
            AgentToolExecutionMode::Parallel => ToolExecutionMode::Parallel,
            AgentToolExecutionMode::Sequential => ToolExecutionMode::Sequential,
        };
        register(
            &mut registry,
            tool_contribution_from_definition(definition, execution_mode)?,
        )?;
    }
    Ok(registry)
}

pub fn tool_contribution_from_definition(
    definition: ToolDefinition,
    execution_mode: ToolExecutionMode,
) -> Result<ToolContribution, BuiltinRegistryError> {
    Ok(ToolContribution {
        id: ContributionId::new(definition.name)?,
        description: definition.description,
        input_schema: definition.input_schema,
        execution_mode,
        handler: None,
        conflict: Default::default(),
    })
}

pub fn command_registry() -> Result<CommandRegistry, BuiltinRegistryError> {
    let mut registry = CommandRegistry::commands();
    for command in COMMANDS {
        register(
            &mut registry,
            CommandContribution {
                id: command_id(command.name)?,
                description: command.summary.into(),
                handler: None,
                conflict: Default::default(),
            },
        )?;
    }
    for (id, description) in [
        (
            "prompt.include",
            "Include a project prompt template by name using /prompt:<name>",
        ),
        (
            "skill.include",
            "Include a global or project skill by name using /skill:<name>",
        ),
    ] {
        register(
            &mut registry,
            CommandContribution {
                id: ContributionId::new(id)?,
                description: description.into(),
                handler: None,
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

pub fn keymap_registry(keymap: &KeymapConfig) -> Result<KeymapRegistry, BuiltinRegistryError> {
    let mut registry = KeymapRegistry::keymaps();
    for info in key_action_rows() {
        register(
            &mut registry,
            KeymapContribution {
                id: ContributionId::new(info.action.id())?,
                action: info.action.id().into(),
                context: info.context.label().into(),
                default_bindings: keymap
                    .bindings_for(info.action)
                    .into_iter()
                    .map(|binding| binding.to_string())
                    .collect(),
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

pub fn hook_registry() -> Result<HookRegistry, BuiltinRegistryError> {
    let mut registry = HookRegistry::hooks();
    for event in builtin_hook_events() {
        register(
            &mut registry,
            HookContribution {
                id: ContributionId::new(format!("hook.{}", hook_event_slug(event)))?,
                event,
                priority: 0,
                mode: HookMode::Observe,
                handler: None,
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

fn builtin_hook_events() -> [HookEventKind; 23] {
    [
        HookEventKind::Startup,
        HookEventKind::ResourceDiscovery,
        HookEventKind::Session,
        HookEventKind::Input,
        HookEventKind::Command,
        HookEventKind::BeforeAgentTurn,
        HookEventKind::AfterAgentTurn,
        HookEventKind::Context,
        HookEventKind::ProviderRequest,
        HookEventKind::ProviderResponse,
        HookEventKind::MessageStream,
        HookEventKind::ToolCall,
        HookEventKind::ToolResult,
        HookEventKind::ToolUpdate,
        HookEventKind::ModelSelection,
        HookEventKind::ThinkingSelection,
        HookEventKind::Compaction,
        HookEventKind::Tree,
        HookEventKind::Reload,
        HookEventKind::Install,
        HookEventKind::Update,
        HookEventKind::Remove,
        HookEventKind::PackageLifecycle,
    ]
}

fn hook_event_slug(event: HookEventKind) -> &'static str {
    match event {
        HookEventKind::Startup => "startup",
        HookEventKind::ResourceDiscovery => "resource_discovery",
        HookEventKind::Session => "session",
        HookEventKind::Input => "input",
        HookEventKind::Command => "command",
        HookEventKind::BeforeAgentTurn => "before_agent_turn",
        HookEventKind::AfterAgentTurn => "after_agent_turn",
        HookEventKind::Context => "context",
        HookEventKind::ProviderRequest => "provider_request",
        HookEventKind::ProviderResponse => "provider_response",
        HookEventKind::MessageStream => "message_stream",
        HookEventKind::ToolCall => "tool_call",
        HookEventKind::ToolResult => "tool_result",
        HookEventKind::ToolUpdate => "tool_update",
        HookEventKind::ModelSelection => "model_selection",
        HookEventKind::ThinkingSelection => "thinking_selection",
        HookEventKind::Compaction => "compaction",
        HookEventKind::Tree => "tree",
        HookEventKind::Reload => "reload",
        HookEventKind::Install => "install",
        HookEventKind::Update => "update",
        HookEventKind::Remove => "remove",
        HookEventKind::PackageLifecycle => "package_lifecycle",
    }
}

pub fn settings_page_registry() -> Result<SettingsPageRegistry, BuiltinRegistryError> {
    let mut registry = SettingsPageRegistry::settings_pages();
    for item in [
        SettingsMenuItem::ModelSelection,
        SettingsMenuItem::ThinkingLevel,
        SettingsMenuItem::CollapseMode,
        SettingsMenuItem::ChatStyle,
        SettingsMenuItem::Tools,
        SettingsMenuItem::Keymaps,
    ] {
        register(
            &mut registry,
            SettingsPageContribution {
                id: ContributionId::new(format!("settings.{}", slug(item.label())))?,
                title: item.label().into(),
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

pub fn theme_registry() -> Result<ThemeRegistry, BuiltinRegistryError> {
    let mut registry = ThemeRegistry::themes();
    for style in ChatStyle::all() {
        register(
            &mut registry,
            ThemeContribution {
                id: ContributionId::new(format!("theme.{}", chat_style_value(style)))?,
                path: format!("builtin://theme/{}", chat_style_value(style)),
                tokens: Default::default(),
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

pub fn provider_registry(
    openrouter_models: impl IntoIterator<Item = String>,
) -> Result<ProviderModelRegistry, BuiltinRegistryError> {
    let mut registry = ProviderModelRegistry::providers_models();
    let openrouter_models = openrouter_models.into_iter().collect::<Vec<_>>();
    for provider in oino_provider_catalog::providers() {
        let mut model_ids = provider
            .default_model()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if provider.id == "openrouter" {
            model_ids.extend(openrouter_models.iter().cloned());
            model_ids.sort();
            model_ids.dedup();
        }
        register(
            &mut registry,
            ProviderContribution {
                id: ContributionId::new(format!("provider.{}", provider.id))?,
                provider_id: provider.id.into(),
                display_name: provider.display_name.into(),
                model_ids,
                privacy: Default::default(),
                runtime: None,
                hook: None,
                conflict: Default::default(),
            },
        )?;
    }
    Ok(registry)
}

pub fn resource_registry(
    resources: &ResourceCatalog,
) -> Result<ResourceRegistry, BuiltinRegistryError> {
    let mut registry = ResourceRegistry::resources();
    register_resource(
        &mut registry,
        "resource.system_prompt",
        ResourceKind::SystemPrompt,
        &resources.paths.global_system_prompt,
    )?;
    register_resource(
        &mut registry,
        "resource.project_instructions",
        ResourceKind::ProjectInstructions,
        &resources.paths.project_agent,
    )?;
    for prompt in &resources.prompts {
        register_resource(
            &mut registry,
            &format!("resource.prompt.{}", slug(&prompt.name)),
            ResourceKind::Prompt,
            &prompt.path,
        )?;
    }
    for skill in &resources.skills {
        register_resource(
            &mut registry,
            &format!("resource.skill.{}", slug(&skill.name)),
            ResourceKind::Skill,
            &skill.path,
        )?;
    }
    Ok(registry)
}

fn register_resource(
    registry: &mut ResourceRegistry,
    id: &str,
    kind: ResourceKind,
    path: &Path,
) -> Result<(), BuiltinRegistryError> {
    register(
        registry,
        ResourceContribution {
            id: ContributionId::new(id)?,
            kind,
            path: path.display().to_string(),
            conflict: Default::default(),
        },
    )
}

fn register<T>(
    registry: &mut oino_extension_core::TypedContributionRegistry<T>,
    contribution: T,
) -> Result<(), BuiltinRegistryError>
where
    T: oino_extension_core::RegistryContribution,
{
    let id = contribution.contribution_id().clone();
    registry.register_entry(
        RegistryEntryKey::new(id.as_str()),
        builtin_metadata(id)?,
        contribution,
    )?;
    Ok(())
}

fn builtin_metadata(id: ContributionId) -> Result<ContributionMetadata, BuiltinRegistryError> {
    Ok(ContributionMetadata::new(id, builtin_source())
        .with_extension_id(ExtensionId::new(BUILTIN_EXTENSION_ID)?)
        .with_lifecycle(LifecycleState::Active))
}

fn builtin_source() -> SourceDescriptor {
    SourceDescriptor {
        scope: SourceScope::BuiltIn,
        kind: SourceKind::BuiltIn,
        path: None,
        registry: Some("oino".into()),
    }
}

fn command_id(name: &str) -> Result<ContributionId, BuiltinRegistryError> {
    ContributionId::new(slug(name.trim_start_matches('/'))).map_err(BuiltinRegistryError::from)
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            output.push(character);
            previous_separator = false;
        } else if matches!(character, '.' | '_' | '-') {
            if !previous_separator && !output.is_empty() {
                output.push(character);
                previous_separator = true;
            }
        } else if !previous_separator && !output.is_empty() {
            output.push('-');
            previous_separator = true;
        }
    }
    while output.ends_with(['.', '_', '-']) {
        output.pop();
    }
    if output.is_empty() {
        "unnamed".into()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_env::{ExecutionEnv, LocalExecutionEnv};
    use oino_tools::{session_title_tool, SESSION_TITLE_TOOL_NAME};
    use std::error::Error;

    #[test]
    fn registry_catalog_represents_builtin_surfaces() -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let paths = oino_resource::ResourcePaths::from_home_and_cwd(temp.path(), temp.path())?;
        paths.ensure_skeleton()?;
        let resources = paths.load_catalog();
        let env = Arc::new(LocalExecutionEnv) as Arc<dyn ExecutionEnv>;
        let mut tools = oino_tools::default_tools(env, temp.path());
        tools.insert(
            SESSION_TITLE_TOOL_NAME.into(),
            session_title_tool(Arc::new(|_, _| Box::pin(async { Ok(()) }))),
        );

        let catalog = BuiltinRegistryCatalog::from_parts(
            &tools,
            &KeymapConfig::default(),
            &resources,
            ["xai/glm-5.1".to_string()],
        )?;

        assert!(catalog.total_contributions() > tools.len());
        let tool_snapshot = catalog
            .tools
            .compose(&oino_extension_core::RegistryPolicy::default());
        let tool_ids = tool_snapshot
            .active
            .iter()
            .map(|entry| entry.effective_id.as_str())
            .collect::<Vec<_>>();
        assert!(tool_ids.contains(&"read"));
        assert!(tool_ids.contains(&"bash"));
        assert!(tool_ids.contains(&"edit"));
        assert!(tool_ids.contains(&"write"));
        assert!(tool_ids.contains(&SESSION_TITLE_TOOL_NAME));
        assert!(catalog.commands.inner().len() >= COMMANDS.len());
        assert!(!catalog.keymaps.inner().is_empty());
        assert_eq!(catalog.hooks.inner().len(), builtin_hook_events().len());
        assert!(!catalog.settings_pages.inner().is_empty());
        assert_eq!(catalog.themes.inner().len(), ChatStyle::all().len());
        assert_eq!(
            catalog.providers.inner().len(),
            oino_provider_catalog::providers().len()
        );
        assert!(catalog.resources.inner().len() >= 2);
        Ok(())
    }

    #[test]
    fn slug_produces_valid_contribution_ids() -> Result<(), Box<dyn Error>> {
        let id = ContributionId::new(format!("resource.prompt.{}", slug("Demo Prompt!")))?;
        assert_eq!(id.as_str(), "resource.prompt.demo-prompt");
        Ok(())
    }

    #[test]
    fn optional_builtin_packages_are_valid_oino_packages() -> Result<(), Box<dyn Error>> {
        assert!(!optional_builtin_packages().is_empty());
        for package in optional_builtin_packages() {
            let package_dir = package.path();
            let manifest_path = package_dir.join(oino_extension_core::PACKAGE_MANIFEST_FILE);
            let manifest = read_json::<oino_extension_core::PackageManifest>(&manifest_path)?;
            manifest.validate()?;
            assert_eq!(manifest.id.as_str(), package.id);
            assert!(!manifest.display_name.trim().is_empty());
            assert!(!manifest.extensions.is_empty());

            for extension in &manifest.extensions {
                let extension_path = package_dir.join(&extension.manifest);
                let extension_manifest =
                    read_json::<oino_extension_core::ExtensionManifest>(&extension_path)?;
                extension_manifest.validate()?;
                assert_eq!(
                    extension_manifest.package_id.as_ref().map(|id| id.as_str()),
                    Some(manifest.id.as_str())
                );
            }
        }
        Ok(())
    }

    #[test]
    fn optional_builtin_package_lookup_accepts_id_or_directory_name() -> Result<(), Box<dyn Error>>
    {
        let by_id = optional_builtin_package_path("oino.footer_status")
            .ok_or("footer package should resolve by id")?;
        let by_dir = optional_builtin_package_path("footer-status")
            .ok_or("footer package should resolve by directory name")?;
        assert_eq!(by_id, by_dir);
        assert!(by_id.ends_with("footer-status"));
        Ok(())
    }

    #[test]
    fn mode_sandbox_skill_documents_profile_configuration() -> Result<(), Box<dyn Error>> {
        let package = optional_builtin_package_path("mode-sandbox")
            .ok_or("mode-sandbox package should resolve")?;
        let skill_path =
            package.join("extensions/mode-sandbox/resources/skills/mode-sandbox/SKILL.md");
        let skill = std::fs::read_to_string(skill_path)?;
        assert!(skill.contains("name: mode-sandbox"));
        assert!(skill.contains("/mode <profile>"));
        assert!(skill.contains(".oino/sandbox-mode"));
        assert!(skill.contains("~/.oino/sandbox-mode"));
        assert!(skill.contains("/mode:create"));
        assert!(skill.contains("avoid removed/reserved names `read` and `create`"));
        Ok(())
    }

    #[test]
    fn craft_skill_resource_is_oino_native_and_has_validation_fixtures(
    ) -> Result<(), Box<dyn Error>> {
        let package = optional_builtin_package_path("craft-skill")
            .ok_or("craft-skill package should resolve")?;
        let skill_path =
            package.join("extensions/craft-skill/resources/skills/craft-skill/SKILL.md");
        let skill = std::fs::read_to_string(skill_path)?;
        assert!(skill.contains("name: craft-skill"));
        assert!(skill.contains("description: Use when"));
        assert!(skill.contains(".oino/skills"));
        assert!(skill.contains("/skill:<skill-name>"));
        assert!(skill.contains("evaluation prompts"));
        let legacy_agent_name = ["Clau", "de"].concat();
        let legacy_provider_name = ["Anth", "ropic"].concat();
        let legacy_code_phrase = format!("{} code", legacy_agent_name.to_ascii_lowercase());
        let legacy_file_name = [legacy_agent_name.as_str(), ".md"].concat();
        for forbidden in [
            legacy_agent_name.as_str(),
            legacy_provider_name.as_str(),
            legacy_code_phrase.as_str(),
            legacy_file_name.as_str(),
        ] {
            assert!(
                !skill.contains(forbidden),
                "craft skill should avoid source-specific wording: {forbidden}"
            );
        }

        let valid_fixture = std::fs::read_to_string(package.join("fixtures/valid-skill/SKILL.md"))?;
        assert!(valid_fixture.contains("name: release-notes"));
        assert!(valid_fixture.contains("description: Use when"));
        let evals = std::fs::read_to_string(package.join("fixtures/eval-prompts.md"))?;
        assert!(evals.contains("Direct trigger should use the skill"));
        assert!(evals.contains("Nearby request should not use the skill"));
        assert!(evals.contains("Messy request should ask or assume carefully"));
        Ok(())
    }

    fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }
}
