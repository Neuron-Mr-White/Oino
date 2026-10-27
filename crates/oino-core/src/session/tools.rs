//! Tool registry, loadout and the base system prompt options (pi `_refreshToolRegistry`,
//! `_applyToolLoadout`, `_rebuildSystemPrompt`, `_preparePromptAndToolLoadout`,
//! `_restoreToolsFromTranscript`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};
use oino_agent::AgentTool;
use oino_ai::{AgentMessage, JsonValue, SystemMessage, UserTextField};
use oino_resources::BuildSystemPromptOptions;
use oino_resources::source::SourceInfo;
use oino_resources::system_prompt::{build_system_prompt_sections, diff_system_prompt_sections};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{AgentSession, current_system_message, lock};
use crate::events::OinoEvent;
use crate::extensions::api::ToolExposure;
use crate::extensions::tool::ExtensionTool;

/// pi `BUILTIN_PATH_PREFIX`.
pub const BUILTIN_PATH_PREFIX: &str = "builtin:";

/// Where a tool comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "id")]
pub enum ToolSource {
    /// Built-in (`oino-tools`).
    Builtin,
    /// An extension.
    Extension(String),
    /// SDK custom tool.
    Sdk,
}

/// A tool with the metadata the session needs.
#[derive(Clone)]
pub struct ToolEntry {
    /// Executable tool.
    pub tool: Arc<dyn AgentTool>,
    /// Prompt snippet (tools without one are not listed in the prompt).
    pub prompt_snippet: Option<String>,
    /// Prompt guidelines.
    pub prompt_guidelines: Vec<String>,
    /// Exposure.
    pub exposure: ToolExposure,
    /// Activated on registration.
    pub default_active: bool,
    /// Origin.
    pub source: ToolSource,
    /// Namespace.
    pub namespace: Option<JsonValue>,
    /// Annotations.
    pub annotations: Option<JsonValue>,
    /// Reachable tools.
    pub reaches_tools: Option<JsonValue>,
}

impl std::fmt::Debug for ToolEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolEntry")
            .field("name", &self.name())
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl ToolEntry {
    /// A built-in tool entry.
    #[must_use]
    pub fn builtin(
        tool: Arc<dyn AgentTool>,
        prompt_snippet: Option<String>,
        prompt_guidelines: Vec<String>,
    ) -> Self {
        Self {
            tool,
            prompt_snippet,
            prompt_guidelines,
            exposure: ToolExposure::Direct,
            default_active: true,
            source: ToolSource::Builtin,
            namespace: None,
            annotations: None,
            reaches_tools: None,
        }
    }

    /// Tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.tool.meta().name
    }

    fn source_info(&self) -> SourceInfo {
        let (path, source) = match &self.source {
            ToolSource::Builtin => (
                format!("{BUILTIN_PATH_PREFIX}{}", self.name()),
                "builtin".to_owned(),
            ),
            ToolSource::Extension(id) => (id.clone(), format!("extension:{id}")),
            ToolSource::Sdk => (format!("<sdk:{}>", self.name()), "sdk".to_owned()),
        };
        SourceInfo::synthetic(&PathBuf::from(path), &source, None, None)
    }
}

/// pi `ToolInfo` (+ Oino `source`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Schema.
    pub parameters: JsonValue,
    /// Prompt guidelines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_guidelines: Option<Vec<String>>,
    /// Exposure.
    pub exposure: ToolExposure,
    /// Namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<JsonValue>,
    /// Annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<JsonValue>,
    /// Reachable tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaches_tools: Option<JsonValue>,
    /// Provenance.
    pub source_info: SourceInfo,
}

/// pi `createToolNameMatcher`: exact names and `*` patterns.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolMatcher {
    names: HashSet<String>,
    patterns: Vec<Regex>,
}

impl ToolMatcher {
    pub(crate) fn new(entries: &[String]) -> Self {
        let names = entries
            .iter()
            .filter(|e| !e.contains('*'))
            .cloned()
            .collect();
        let patterns = entries
            .iter()
            .filter(|e| e.contains('*'))
            .filter_map(|pattern| {
                let source = pattern
                    .split('*')
                    .map(regex::escape)
                    .collect::<Vec<_>>()
                    .join(".*");
                Regex::new(&format!("^{source}$")).ok()
            })
            .collect();
        Self { names, patterns }
    }

    pub(crate) fn matches(&self, name: &str) -> bool {
        self.names.contains(name) || self.patterns.iter().any(|p| p.is_match(name))
    }
}

/// Tool bookkeeping of a session.
pub(crate) struct ToolState {
    builtin: Vec<ToolEntry>,
    custom: Vec<ToolEntry>,
    allowed: Option<ToolMatcher>,
    excluded: ToolMatcher,
    registry: IndexMap<String, ToolEntry>,
    pending: IndexSet<String>,
    hidden: HashSet<String>,
    pub(crate) base_options: BuildSystemPromptOptions,
}

impl ToolState {
    pub(crate) fn new(
        builtin: Vec<ToolEntry>,
        custom: Vec<ToolEntry>,
        allowed: Option<&[String]>,
        excluded: &[String],
    ) -> Self {
        Self {
            builtin,
            custom,
            allowed: allowed.map(ToolMatcher::new),
            excluded: ToolMatcher::new(excluded),
            registry: IndexMap::new(),
            pending: IndexSet::new(),
            hidden: HashSet::new(),
            base_options: BuildSystemPromptOptions::default(),
        }
    }

    fn is_allowed(&self, name: &str) -> bool {
        !self.excluded.matches(name) && self.allowed.as_ref().is_none_or(|a| a.matches(name))
    }

    fn exposure(&self, name: &str) -> ToolExposure {
        self.registry
            .get(name)
            .map_or(ToolExposure::Direct, |e| e.exposure)
    }

    fn is_declarable(&self, name: &str) -> bool {
        matches!(
            self.exposure(name),
            ToolExposure::Direct | ToolExposure::ModelOnly
        )
    }

    fn activated_on_registration(&self, name: &str) -> bool {
        self.is_declarable(name) && self.registry.get(name).is_some_and(|e| e.default_active)
    }

    pub(crate) fn hidden(&self) -> &HashSet<String> {
        &self.hidden
    }
}

impl AgentSession {
    /// Names of the active tools (declared to the model).
    #[must_use]
    pub fn active_tool_names(&self) -> Vec<String> {
        self.inner
            .agent
            .with_state(|state| state.tools.iter().map(|t| t.meta().name.clone()).collect())
    }

    /// Tools callable through nested calls: active `direct` tools plus `codemode`/`deferred`.
    #[must_use]
    pub fn callable_tool_names(&self) -> Vec<String> {
        self.callable_tools(&self.active_tool_names().into_iter().collect())
            .iter()
            .map(|t| t.meta().name.clone())
            .collect()
    }

    pub(crate) fn callable_tools(&self, active: &HashSet<String>) -> Vec<Arc<dyn AgentTool>> {
        let tools = lock(&self.inner.tools);
        tools
            .registry
            .values()
            .filter(|entry| match entry.exposure {
                ToolExposure::Codemode | ToolExposure::Deferred => true,
                ToolExposure::Direct => active.contains(entry.name()),
                ToolExposure::ModelOnly | ToolExposure::Hidden => false,
            })
            .map(|entry| Arc::clone(&entry.tool))
            .collect()
    }

    /// Every registered tool (pi `getAllTools`).
    #[must_use]
    pub fn all_tools(&self) -> Vec<ToolInfo> {
        let tools = lock(&self.inner.tools);
        tools
            .registry
            .values()
            .map(|entry| {
                let meta = entry.tool.meta();
                ToolInfo {
                    name: meta.name.clone(),
                    description: meta.description.clone(),
                    parameters: meta.parameters.clone(),
                    prompt_guidelines: (!entry.prompt_guidelines.is_empty())
                        .then(|| entry.prompt_guidelines.clone()),
                    exposure: entry.exposure,
                    namespace: entry.namespace.clone(),
                    annotations: entry.annotations.clone(),
                    reaches_tools: entry.reaches_tools.clone(),
                    source_info: entry.source_info(),
                }
            })
            .collect()
    }

    /// pi `setActiveToolsByName`.
    pub fn set_active_tools_by_name(&self, names: &[String]) {
        let previous = self.active_tool_names();
        self.set_active_tools(names);
        let active: HashSet<String> = self.active_tool_names().into_iter().collect();
        if previous.iter().any(|name| !active.contains(name)) {
            lock(&self.inner.tools).pending.clear();
        }
    }

    pub(crate) fn set_active_tools(&self, names: &[String]) {
        let tools = self.apply_tool_loadout(names);
        {
            let mut state = lock(&self.inner.tools);
            for tool in &tools {
                state.pending.shift_remove(&tool.meta().name);
            }
        }
        let names: Vec<String> = tools.iter().map(|t| t.meta().name.clone()).collect();
        self.rebuild_system_prompt(&names);
        self.inner
            .hub
            .publish(OinoEvent::ToolsChanged { active: names });
    }

    /// pi `_applyToolLoadout`: sets the agent's tools to the registered, non-hidden ones.
    pub(crate) fn apply_tool_loadout(&self, names: &[String]) -> Vec<Arc<dyn AgentTool>> {
        let mut seen = HashSet::new();
        let tools: Vec<Arc<dyn AgentTool>> = {
            let state = lock(&self.inner.tools);
            names
                .iter()
                .filter(|name| seen.insert((*name).clone()))
                .filter_map(|name| {
                    let entry = state.registry.get(name)?;
                    (entry.exposure != ToolExposure::Hidden).then(|| Arc::clone(&entry.tool))
                })
                .collect()
        };
        lock(&self.inner.tools).hidden.clear();
        self.inner.agent.set_tools(tools.clone());
        tools
    }

    /// pi `_rebuildSystemPrompt`: recomputes the base prompt options.
    pub(crate) fn rebuild_system_prompt(&self, names: &[String]) {
        let (snippets, guidelines, valid, hidden) = {
            let state = lock(&self.inner.tools);
            let snippets: IndexMap<String, String> = state
                .registry
                .values()
                .filter_map(|entry| {
                    normalize_snippet(entry.prompt_snippet.as_deref())
                        .map(|snippet| (entry.name().to_owned(), snippet))
                })
                .collect();
            let guidelines: IndexMap<String, Vec<String>> = state
                .registry
                .values()
                .filter_map(|entry| {
                    let normalized = normalize_guidelines(&entry.prompt_guidelines);
                    (!normalized.is_empty()).then(|| (entry.name().to_owned(), normalized))
                })
                .collect();
            let valid: Vec<String> = names
                .iter()
                .filter(|name| state.registry.contains_key(*name))
                .cloned()
                .collect();
            (
                snippets,
                guidelines,
                valid,
                state.hidden.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let resources = self
            .inner
            .resources
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut options = BuildSystemPromptOptions {
            custom_prompt: resources.system_prompt().map(str::to_owned),
            force_system_prompt: None,
            selected_tools: valid,
            hidden_tools: hidden,
            tool_snippets: snippets,
            tool_guidelines: guidelines,
            prompt_guidelines: Vec::new(),
            append_system_prompt: resources.append_system_prompt().join("\n\n"),
            sections: IndexMap::new(),
            cwd: self.inner.cwd.display().to_string(),
            context_files: resources.context_files().to_vec(),
            skills: resources.skills().to_vec(),
        };
        drop(resources);
        self.apply_skill_opt_in(&mut options.skills);
        if let Some(link) = &self.inner.agents.link {
            match &link.prompt {
                crate::agents::SystemPromptSpec::Default => {}
                crate::agents::SystemPromptSpec::Replace(prompt) => {
                    options.custom_prompt = Some(prompt.clone());
                }
                crate::agents::SystemPromptSpec::Append(extra) => {
                    if options.append_system_prompt.is_empty() {
                        options.append_system_prompt.clone_from(extra);
                    } else {
                        options.append_system_prompt =
                            format!("{}\n\n{extra}", options.append_system_prompt);
                    }
                }
            }
        }
        lock(&self.inner.tools).base_options = options;
    }

    /// Turns opt-in (`default-enabled: false`) skills on or off from the current
    /// `extensionSettings.skills.states` (read at every rebuild and run, so a toggle in
    /// `/skills` applies from the next prompt).
    pub(crate) fn apply_skill_opt_in(&self, skills: &mut [oino_resources::Skill]) {
        if skills.iter().all(|skill| skill.default_enabled) {
            return;
        }
        let states = lock(&self.inner.settings)
            .settings()
            .extension_settings(oino_resources::skills::SKILL_STATES_NAMESPACE, None);
        oino_resources::skills::apply_opt_in(skills, &states);
    }

    /// Base prompt options (pi `getSystemPromptOptions`).
    #[must_use]
    pub fn system_prompt_options(&self) -> BuildSystemPromptOptions {
        lock(&self.inner.tools).base_options.clone()
    }

    /// Current effective system prompt, including changes not yet sent (pi `systemPrompt`).
    #[must_use]
    pub fn system_prompt(&self) -> String {
        let options = lock(&self.inner.book)
            .run_prompt_options
            .clone()
            .unwrap_or_else(|| self.system_prompt_options());
        oino_resources::system_prompt::build_system_prompt(&options, &self.inner.docs_root)
            .unwrap_or_default()
    }

    /// pi `_preparePromptAndToolLoadout`: applies `options` and returns the system patch.
    pub(crate) fn prepare_prompt_and_tool_loadout(
        &self,
        options: &mut BuildSystemPromptOptions,
        messages: &[AgentMessage],
    ) -> Option<SystemMessage> {
        options.selected_tools = self
            .apply_tool_loadout(&options.selected_tools)
            .iter()
            .map(|t| t.meta().name.clone())
            .collect();
        options.hidden_tools = lock(&self.inner.tools).hidden().iter().cloned().collect();
        let previous: IndexMap<String, String> = current_system_message(messages)
            .and_then(|m| m.sections)
            .map(|sections| {
                sections
                    .into_iter()
                    .filter_map(|(name, text)| text.map(|text| (name, text)))
                    .collect()
            })
            .unwrap_or_default();
        let sections = match build_system_prompt_sections(options, &self.inner.docs_root) {
            Ok(sections) => sections,
            Err(error) => {
                self.runner()
                    .emit_error("<runtime>", "system_prompt", error.to_string());
                return None;
            }
        };
        diff_system_prompt_sections(&previous, &sections).map(|sections| SystemMessage {
            content: UserTextField::Text(String::new()),
            sections: Some(sections),
            tools_added: None,
            tools_removed: None,
            timestamp: oino_agent::now_ms(),
        })
    }

    /// pi `_refreshToolRegistry`.
    pub(crate) fn refresh_tool_registry(
        &self,
        active: Option<Vec<String>>,
        include_all_extension_tools: bool,
    ) {
        let runner = self.runner();
        let cx = runner.call_context().clone();
        let previous_active = self.active_tool_names();
        let mut state = lock(&self.inner.tools);
        let previous_activated: HashSet<String> = state
            .registry
            .keys()
            .filter(|name| state.activated_on_registration(name))
            .cloned()
            .collect();
        let mut registry: IndexMap<String, ToolEntry> = IndexMap::new();
        for entry in state.builtin.iter().filter(|e| state.is_allowed(e.name())) {
            registry.insert(entry.name().to_owned(), entry.clone());
        }
        let mut extension_names = Vec::new();
        for registered in runner.tools() {
            if !state.is_allowed(&registered.spec.name) {
                continue;
            }
            let spec = registered.spec.clone();
            let entry = ToolEntry {
                tool: Arc::new(ExtensionTool::new(
                    spec.clone(),
                    Arc::clone(&registered.extension),
                    &cx,
                )),
                prompt_snippet: spec.prompt_snippet.clone(),
                prompt_guidelines: spec.prompt_guidelines.clone(),
                exposure: spec.exposure,
                default_active: spec.default_active.unwrap_or(true),
                source: ToolSource::Extension(registered.extension.instance.id().to_owned()),
                namespace: spec.namespace.clone(),
                annotations: spec.annotations.clone(),
                reaches_tools: spec.reaches_tools.clone(),
            };
            extension_names.push(spec.name.clone());
            registry.insert(spec.name, entry);
        }
        for entry in state.custom.iter().filter(|e| state.is_allowed(e.name())) {
            extension_names.push(entry.name().to_owned());
            registry.insert(entry.name().to_owned(), entry.clone());
        }
        state.registry = registry;
        let explicit = active.is_some();
        let mut next: Vec<String> = active
            .unwrap_or(previous_active)
            .into_iter()
            .filter(|name| state.is_allowed(name))
            .collect();
        if let Some(allowed) = state.allowed.clone() {
            for name in state.registry.keys() {
                if allowed.matches(name) && state.is_declarable(name) {
                    next.push(name.clone());
                }
            }
        } else if include_all_extension_tools {
            for name in &extension_names {
                if state.activated_on_registration(name) {
                    next.push(name.clone());
                }
            }
        } else if !explicit {
            for name in state.registry.keys() {
                if !previous_activated.contains(name) && state.activated_on_registration(name) {
                    next.push(name.clone());
                }
            }
        }
        next.extend(state.pending.iter().cloned());
        drop(state);
        let mut seen = HashSet::new();
        next.retain(|name| seen.insert(name.clone()));
        self.set_active_tools(&next);
    }

    /// pi `_restoreToolsFromTranscript`.
    pub(crate) fn restore_tools_from_transcript(&self, modifiers: &[String]) {
        lock(&self.inner.tools).pending.clear();
        let systems: Vec<SystemMessage> = lock(&self.inner.manager)
            .branch(None)
            .into_iter()
            .filter_map(|entry| match entry.message() {
                Some(AgentMessage::System(system)) => Some(system.clone()),
                _ => None,
            })
            .collect();
        if systems.is_empty() {
            return;
        }
        let mentioned: HashSet<String> = systems
            .iter()
            .flat_map(|m| {
                m.tools_added
                    .iter()
                    .flatten()
                    .map(|t| t.name.clone())
                    .chain(m.tools_removed.iter().flatten().map(|t| t.name.clone()))
            })
            .collect();
        let recorded: Vec<String> = oino_ai::transcript::SystemState::fold(systems.iter())
            .tools
            .into_iter()
            .map(|t| t.name)
            .collect();
        let names = {
            let state = lock(&self.inner.tools);
            let builtin: HashSet<&str> = state.builtin.iter().map(ToolEntry::name).collect();
            let new_tools: Vec<String> = state
                .registry
                .keys()
                .filter(|name| {
                    !mentioned.contains(*name)
                        && !builtin.contains(name.as_str())
                        && state.activated_on_registration(name)
                })
                .cloned()
                .collect();
            oino_settings::tools::apply_tool_modifiers(&[recorded, new_tools].concat(), modifiers)
        };
        {
            let mut state = lock(&self.inner.tools);
            let pending: IndexSet<String> = names
                .iter()
                .filter(|n| state.is_allowed(n))
                .cloned()
                .collect();
            state.pending = pending;
        }
        self.set_active_tools(&names);
    }

    /// Hidden declarations (`prepareLoadout`).
    pub(crate) fn hidden_declarations(&self) -> HashSet<String> {
        lock(&self.inner.tools).hidden().clone()
    }

    /// Clears pending (not yet registered) restored tools.
    pub(crate) fn clear_pending_tools(&self) {
        lock(&self.inner.tools).pending.clear();
    }
}

/// pi `_normalizePromptSnippet`.
fn normalize_snippet(text: Option<&str>) -> Option<String> {
    let text = text?;
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then_some(one_line)
}

/// pi `_normalizePromptGuidelines`.
fn normalize_guidelines(guidelines: &[String]) -> Vec<String> {
    let mut unique = IndexSet::new();
    for guideline in guidelines {
        let normalized = guideline.trim();
        if !normalized.is_empty() {
            unique.insert(normalized.to_owned());
        }
    }
    unique.into_iter().collect()
}
