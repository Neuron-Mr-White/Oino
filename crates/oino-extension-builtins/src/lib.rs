#![doc = r#"Built-in Oino contribution catalogs backed by the extension registry model.

This crate is the bridge between hardcoded Oino surfaces and the generic
extension-kernel contribution registries. It does not execute tools or render
UI itself; it only represents built-in tools, commands, keymaps, resources,
settings pages, themes, and provider metadata as registry contributions.
"#]
#![forbid(unsafe_code)]

use oino_agent_loop::{Tool, ToolDefinition, ToolExecutionMode as AgentToolExecutionMode};
use oino_extension_core::{
    CommandContribution, CommandRegistry, ContributionId, ContributionMetadata, ExtensionCoreError,
    ExtensionId, KeymapContribution, KeymapRegistry, LifecycleState, ProviderContribution,
    ProviderModelRegistry, RegistryEntryKey, RegistryValidationError, ResourceContribution,
    ResourceKind, ResourceRegistry, SettingsPageContribution, SettingsPageRegistry,
    SourceDescriptor, SourceKind, SourceScope, ThemeContribution, ThemeRegistry, ToolContribution,
    ToolExecutionMode, ToolRegistry,
};
use oino_resource::ResourceCatalog;
use oino_tui::{
    chat_style_value, key_action_rows, ChatStyle, KeymapConfig, SettingsMenuItem, COMMANDS,
};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use thiserror::Error;

pub const BUILTIN_EXTENSION_ID: &str = "oino.builtins";

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
    register(
        &mut registry,
        ProviderContribution {
            id: ContributionId::new("provider.openrouter")?,
            provider_id: "openrouter".into(),
            model_ids: openrouter_models.into_iter().collect(),
            conflict: Default::default(),
        },
    )?;
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
        assert!(!catalog.settings_pages.inner().is_empty());
        assert_eq!(catalog.themes.inner().len(), ChatStyle::all().len());
        assert_eq!(catalog.providers.inner().len(), 1);
        assert!(catalog.resources.inner().len() >= 2);
        Ok(())
    }

    #[test]
    fn slug_produces_valid_contribution_ids() -> Result<(), Box<dyn Error>> {
        let id = ContributionId::new(format!("resource.prompt.{}", slug("Demo Prompt!")))?;
        assert_eq!(id.as_str(), "resource.prompt.demo-prompt");
        Ok(())
    }
}
