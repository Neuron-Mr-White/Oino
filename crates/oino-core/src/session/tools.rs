//! Tool registry, loadout and the base system prompt options (`_refreshToolRegistry`,
//! `_applyToolLoadout`, `_rebuildSystemPrompt`, `_preparePromptAndToolLoadout`,
//! `_restoreToolsFromTranscript`).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};
use oino_agent::AgentTool;
use oino_ai::{AgentMessage, JsonValue, SystemMessage};
use oino_resources::BuildSystemPromptOptions;
use oino_resources::source::SourceInfo;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{AgentSession, lock};
use crate::events::OinoEvent;
use crate::extensions::api::ToolExposure;
use crate::extensions::tool::ExtensionTool;

/// Path prefix of built-in tools.
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

/// `ToolInfo` (+ Oino `source`).
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

/// Exact names and `*` patterns.
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
    /// Frozen metadata, in declaration order. Execution always resolves the live registry.
    declared: Vec<oino_agent::ToolDefinitionMeta>,
    initialized: bool,
    catalog: Vec<String>,
    codemode_only: bool,
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
            declared: Vec::new(),
            initialized: false,
            catalog: Vec::new(),
            codemode_only: false,
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
}

impl AgentSession {
    /// Names of the active tools (declared to the model).
    #[must_use]
    pub fn active_tool_names(&self) -> Vec<String> {
        self.inner
            .agent
            .with_state(|state| state.tools.iter().map(|t| t.meta().name.clone()).collect())
    }

    /// Tools callable through nested calls: available direct, codemode and deferred tools.
    #[must_use]
    pub fn callable_tool_names(&self) -> Vec<String> {
        self.callable_tools(&self.active_tool_names().into_iter().collect())
            .iter()
            .map(|t| t.meta().name.clone())
            .collect()
    }

    pub(crate) fn callable_tools(&self, _active: &HashSet<String>) -> Vec<Arc<dyn AgentTool>> {
        let patch_allowed =
            patch_eligible(self.model().as_ref().map_or("", |model| model.id.as_str()));
        let tools = lock(&self.inner.tools);
        tools
            .registry
            .values()
            .filter(|entry| entry.name() != "apply_patch" || patch_allowed)
            .filter(|entry| match entry.exposure {
                ToolExposure::Codemode | ToolExposure::Deferred | ToolExposure::Direct => true,
                ToolExposure::ModelOnly | ToolExposure::Hidden => false,
            })
            .map(|entry| Arc::clone(&entry.tool))
            .collect()
    }

    /// Every registered tool.
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

    /// Sets the active tools by name.
    pub fn set_active_tools_by_name(&self, names: &[String]) {
        let previous = self.active_tool_names();
        self.set_active_tools(names);
        let active: HashSet<String> = self.active_tool_names().into_iter().collect();
        if previous.iter().any(|name| !active.contains(name)) {
            lock(&self.inner.tools).pending.clear();
        }
    }

    /// Adds `enable` to and removes `disable` from the CURRENT active tools in one step,
    /// touching nothing else (`disable` applies first, so a name in both ends up active).
    /// Unregistered names in `enable` are ignored like in [`Self::set_active_tools_by_name`];
    /// pending (not yet registered) restored tools stay pending unless named in `disable`.
    /// No-op when the set does not change.
    pub fn update_active_tools(&self, enable: &[String], disable: &[String]) {
        let previous = self.active_tool_names();
        let mut next: Vec<String> = previous
            .iter()
            .filter(|name| !disable.contains(*name) || enable.contains(*name))
            .cloned()
            .collect();
        {
            let mut state = lock(&self.inner.tools);
            for name in disable.iter().filter(|n| !enable.contains(n)) {
                state.pending.shift_remove(name);
            }
            for name in enable {
                let activatable = state
                    .registry
                    .get(name)
                    .is_some_and(|entry| entry.exposure != ToolExposure::Hidden);
                if activatable && !next.contains(name) {
                    next.push(name.clone());
                }
            }
        }
        if next == previous {
            return;
        }
        self.set_active_tools(&next);
    }

    /// An extension's whole-list `set-active-tools`: like [`Self::set_active_tools_by_name`],
    /// except that an active built-in tool (read/bash/edit/write/…) missing from `names` is
    /// kept. Extensions read-modify-write the list to toggle their own tools; a read that
    /// came back empty or partial must not wipe the core tools for the rest of the session.
    /// Built-ins leave only when named in [`Self::update_active_tools`]'s `disable` (or by a
    /// user/frontend whole-list write through [`Self::set_active_tools_by_name`]).
    pub(crate) fn set_active_tools_from_extension(&self, extension: &str, names: &[String]) {
        let previous = self.active_tool_names();
        let kept: Vec<String> = {
            let state = lock(&self.inner.tools);
            previous
                .iter()
                .filter(|name| {
                    !names.contains(*name)
                        && state.builtin.iter().any(|e| e.name() == name.as_str())
                })
                .cloned()
                .collect()
        };
        if kept.is_empty() {
            self.set_active_tools_by_name(names);
            return;
        }
        tracing::warn!(
            extension,
            kept = ?kept,
            "set-active-tools would drop built-in tools; keeping them (disable built-ins by name)"
        );
        let mut next = kept;
        next.extend(names.iter().cloned());
        self.set_active_tools_by_name(&next);
    }

    pub(crate) fn set_active_tools(&self, names: &[String]) {
        let only = lock(&self.inner.settings)
            .snapshot()
            .merged()
            .get("codemode")
            .and_then(|v| v.get("mode"))
            .and_then(JsonValue::as_str)
            == Some("only");
        {
            let mut state = lock(&self.inner.tools);
            if !state.initialized {
                state.codemode_only = only && state.registry.contains_key("codemode");
            }
        }
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

    /// Sets the agent's tools to the registered, non-hidden ones.
    #[allow(
        clippy::too_many_lines,
        reason = "append-only declaration restoration and suffix loading share one registry lock"
    )]
    pub(crate) fn apply_tool_loadout(&self, names: &[String]) -> Vec<Arc<dyn AgentTool>> {
        let budget = lock(&self.inner.settings)
            .snapshot()
            .merged()
            .get("codemode")
            .and_then(|v| v.get("inlineBudget"))
            .and_then(JsonValue::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(3000)
            .saturating_mul(4);
        let mut state = lock(&self.inner.tools);
        if state.declared.is_empty() {
            let recorded = lock(&self.inner.manager)
                .branch(None)
                .iter()
                .filter_map(|entry| match &entry.kind {
                    oino_session::EntryKind::Custom {
                        custom_type,
                        data: Some(data),
                    } if custom_type == "oino.tool-declarations" => {
                        serde_json::from_value::<Vec<oino_ai::ToolDecl>>(data.clone()).ok()
                    }
                    _ => None,
                })
                .next_back();
            if let Some(recorded) = recorded {
                // Old Superagent sessions persisted dotted tool declarations. Replace them
                // only when the renamed implementation exists, preserving other frozen tools.
                let recorded = migrate_superagent_declarations(recorded, |name| {
                    state.registry.contains_key(name)
                });
                state.declared = recorded
                    .into_iter()
                    .map(|decl| {
                        let mut meta = oino_agent::ToolDefinitionMeta::new(
                            &decl.name,
                            &decl.name,
                            decl.description,
                            decl.parameters,
                        );
                        meta.constrained_sampling = decl.constrained_sampling;
                        if let Some(entry) = state.registry.get(&meta.name) {
                            meta.execution_mode = entry.tool.meta().execution_mode;
                            meta.output_schema
                                .clone_from(&entry.tool.meta().output_schema);
                            meta.replay = entry.tool.meta().replay;
                        }
                        meta
                    })
                    .collect();
                state.initialized = true;
            }
        }
        let previous = state
            .declared
            .iter()
            .map(oino_agent::ToolDefinitionMeta::declaration)
            .collect::<Vec<_>>();
        let eligible = patch_eligible(self.model().as_ref().map_or("", |m| m.id.as_str()));
        let mut requested: Vec<String> = names.to_vec();
        if !state.initialized {
            // Registry order is built-ins first, then lexically sorted extensions.
            requested = state
                .registry
                .values()
                .filter(|e| {
                    matches!(e.exposure, ToolExposure::Direct | ToolExposure::ModelOnly)
                        && (!state.codemode_only || matches!(e.name(), "codemode" | "tool_search"))
                        && !is_mcp(e)
                        && (e.name() != "apply_patch" || eligible)
                })
                .map(|e| e.name().to_owned())
                .collect();
            state.initialized = true;
        }
        if eligible && state.registry.contains_key("apply_patch") {
            requested.push("apply_patch".into());
        }
        for name in requested {
            if state.codemode_only && !matches!(name.as_str(), "codemode" | "tool_search") {
                continue;
            }
            if state.declared.iter().any(|m| m.name == name) {
                continue;
            }
            if name == "apply_patch" && !eligible {
                continue;
            }
            if let Some(entry) = state.registry.get(&name)
                && entry.exposure != ToolExposure::Hidden
            {
                let mut meta = entry.tool.meta().clone();
                if name == "codemode" {
                    append_codemode_declarations(&mut meta, &state, budget);
                } else if state.registry.contains_key("codemode")
                    && entry.exposure == ToolExposure::Direct
                {
                    let _ = write!(
                        meta.description,
                        "\nAlso callable from codemode as tools.{}(args).",
                        js_identifier(&name)
                    );
                }
                state.declared.push(meta);
            }
        }
        let declared = state.declared.clone();
        state.hidden.clear();
        drop(state);
        let current = declared
            .iter()
            .map(oino_agent::ToolDefinitionMeta::declaration)
            .collect::<Vec<_>>();
        if previous != current
            && let Err(error) = lock(&self.inner.manager)
                .append_custom_entry("oino.tool-declarations", Some(serde_json::json!(current)))
        {
            self.runner()
                .emit_error("<runtime>", "tool_declarations", error.to_string());
        }
        let tools: Vec<Arc<dyn AgentTool>> = declared
            .into_iter()
            .map(|meta| {
                Arc::new(DeclaredTool {
                    meta,
                    inner: Arc::downgrade(&self.inner),
                }) as Arc<dyn AgentTool>
            })
            .collect();
        self.inner.agent.set_tools(tools.clone());
        tools
    }

    /// Compaction-only declaration rebuild. Keep loaded, available tools, drop
    /// unavailable declarations, and restore built-in/lexical-extension order.
    /// wp-compaction calls this after persisting its new tail view. Slot bytes
    /// are independent and must not be reset. Outside compaction use suffix loads.
    pub fn rebuild_declarations_after_compaction(&self) {
        let mut state = lock(&self.inner.tools);
        let loaded = state
            .declared
            .iter()
            .map(|m| m.name.clone())
            .collect::<HashSet<_>>();
        state.declared = state
            .registry
            .values()
            .filter(|entry| loaded.contains(entry.name()))
            .map(|entry| {
                state
                    .declared
                    .iter()
                    .find(|meta| meta.name == entry.name())
                    .cloned()
                    .unwrap_or_else(|| entry.tool.meta().clone())
            })
            .collect();
        let current = state
            .declared
            .iter()
            .map(oino_agent::ToolDefinitionMeta::declaration)
            .collect::<Vec<_>>();
        drop(state);
        if let Err(error) = lock(&self.inner.manager)
            .append_custom_entry("oino.tool-declarations", Some(serde_json::json!(current)))
        {
            self.runner()
                .emit_error("<runtime>", "tool_declarations", error.to_string());
        }
        self.apply_tool_loadout(&[]);
    }

    /// Recomputes the base prompt options.
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

    /// Base prompt options.
    #[must_use]
    pub fn system_prompt_options(&self) -> BuildSystemPromptOptions {
        lock(&self.inner.tools).base_options.clone()
    }

    /// Current effective system prompt, including changes not yet sent.
    #[must_use]
    pub fn system_prompt(&self) -> String {
        self.frozen_prompt().unwrap_or_else(|| {
            oino_resources::system_prompt::build_system_prompt(
                &self.system_prompt_options(),
                &self.inner.docs_root,
            )
            .unwrap_or_default()
        })
    }

    /// v3 never emits section patches. Declarations may only gain suffixes.
    pub(crate) fn prepare_prompt_and_tool_loadout(
        &self,
        options: &mut BuildSystemPromptOptions,
        _messages: &[AgentMessage],
    ) -> Option<SystemMessage> {
        options.selected_tools = self
            .apply_tool_loadout(&options.selected_tools)
            .iter()
            .map(|t| t.meta().name.clone())
            .collect();
        None
    }

    /// Refreshes the tool registry.
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
        let mut ordered = IndexMap::new();
        for entry in &state.builtin {
            if let Some(value) = registry.shift_remove(entry.name()) {
                ordered.insert(entry.name().to_owned(), value);
            }
        }
        registry.sort_keys();
        ordered.extend(registry);
        let catalog: Vec<String> = ordered.keys().cloned().collect();
        let changed = state.initialized && state.catalog != catalog;
        state.catalog = catalog;
        state.registry = ordered;
        let explicit = active.is_some();
        let mut next: Vec<String> = active
            .unwrap_or(previous_active)
            .into_iter()
            .filter(|name| state.is_allowed(name))
            .collect();
        if let Some(allowed) = state.allowed.clone().filter(|_| !state.initialized) {
            for name in state.registry.keys() {
                if allowed.matches(name) && state.is_declarable(name) {
                    next.push(name.clone());
                }
            }
        } else if include_all_extension_tools && !state.initialized {
            for name in &extension_names {
                if state.activated_on_registration(name) {
                    next.push(name.clone());
                }
            }
        } else if !explicit && !state.initialized {
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
        if changed
            && self.frozen_prompt().is_some()
            && let Err(error) = self.append_reminder("oino", "oino/tools", &self.tool_catalog())
        {
            self.runner()
                .emit_error("<runtime>", "append_reminder", error.to_string());
        }
    }

    /// Callable sandbox metadata. Hidden/model-only tools never enter the sandbox.
    #[must_use]
    pub fn codemode_catalog(&self) -> Vec<JsonValue> {
        let callable: HashSet<String> = self.callable_tool_names().into_iter().collect();
        self.all_tools()
            .into_iter()
            .filter(|tool| callable.contains(&tool.name) && tool.name != "codemode")
            .filter_map(|tool| serde_json::to_value(tool).ok())
            .collect()
    }

    /// Load exact selections atomically, or the first name-keyword matches. Existing
    /// declarations are never replaced and unknown exact names load nothing.
    ///
    /// # Errors
    /// Rejects empty queries and unknown, hidden or unavailable exact selections.
    pub fn search_deferred_tools(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<JsonValue, oino_agent::ToolError> {
        let query = query.trim();
        if query.is_empty() || limit == Some(0) {
            return Err(oino_agent::ToolError::new(
                "A query and positive max_results are required",
            ));
        }
        let state = lock(&self.inner.tools);
        let available = |name: &str| {
            state
                .registry
                .get(name)
                .is_some_and(|e| e.exposure != ToolExposure::Hidden)
        };
        let selected = if let Some(names) = query.strip_prefix("select:") {
            let mut selected = Vec::new();
            for name in names.split(',').map(str::trim) {
                if name.is_empty() || !available(name) {
                    return Err(oino_agent::ToolError::new(format!(
                        "tool {name} is unavailable"
                    )));
                }
                if !selected.iter().any(|n| n == name) {
                    selected.push(name.to_owned());
                }
            }
            selected
        } else {
            let words: Vec<_> = query
                .to_lowercase()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            state
                .registry
                .keys()
                .filter(|name| {
                    available(name) && words.iter().all(|word| name.to_lowercase().contains(word))
                })
                .take(limit.unwrap_or(5))
                .cloned()
                .collect()
        };
        drop(state);
        self.update_active_tools(&selected, &[]);
        Ok(serde_json::json!(
            self.all_tools()
                .into_iter()
                .filter(|tool| selected.contains(&tool.name))
                .collect::<Vec<_>>()
        ))
    }

    /// Loaded deferred tools for wp-compaction's persisted state.
    #[must_use]
    pub fn loaded_deferred_tool_names(&self) -> Vec<String> {
        let state = lock(&self.inner.tools);
        state
            .declared
            .iter()
            .filter(|meta| {
                state.registry.get(&meta.name).is_some_and(|entry| {
                    is_mcp(entry)
                        || matches!(
                            entry.exposure,
                            ToolExposure::Deferred | ToolExposure::Codemode
                        )
                })
            })
            .map(|meta| meta.name.clone())
            .collect()
    }

    /// Unified catalog for the immutable start slot and later tail updates.
    pub(crate) fn tool_catalog(&self) -> String {
        let state = lock(&self.inner.tools);
        let names = state
            .registry
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let deferred = state
            .registry
            .keys()
            .filter(|name| !state.declared.iter().any(|m| &m.name == *name))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Unified tool catalog:\n{names}\n\nLoad deferred schemas with tool_search using select:<name> or keywords. Never invent tool names.\n\nDeferred tools:\n{deferred}"
        )
    }

    /// Restores the active tool set from the transcript.
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
        let mut recorded: Vec<String> = oino_ai::transcript::SystemState::fold(systems.iter())
            .tools
            .into_iter()
            .map(|t| t.name)
            .collect();
        let default_tools = lock(&self.inner.settings)
            .snapshot()
            .default_tools()
            .unwrap_or_else(|| {
                oino_settings::tools::DEFAULT_TOOL_NAMES
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            });
        let names = {
            let state = lock(&self.inner.tools);
            let builtin: HashSet<&str> = state.builtin.iter().map(ToolEntry::name).collect();
            // A transcript whose loadout holds no built-in at all was written by a runtime
            // that had none registered (the desktop hub before built-ins were defaulted, or a
            // session cloned from pi whose next Oino turn recorded them as removed): restore
            // the default built-ins instead of carrying the loss forward forever.
            if !builtin.is_empty() && !recorded.iter().any(|n| builtin.contains(n.as_str())) {
                let missing: Vec<String> = default_tools
                    .iter()
                    .filter(|n| builtin.contains(n.as_str()))
                    .cloned()
                    .collect();
                recorded.splice(0..0, missing);
            }
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

    /// Clears pending (not yet registered) restored tools.
    pub(crate) fn clear_pending_tools(&self) {
        lock(&self.inner.tools).pending.clear();
    }
}

fn migrate_superagent_declarations(
    declarations: Vec<oino_ai::ToolDecl>,
    registered: impl Fn(&str) -> bool,
) -> Vec<oino_ai::ToolDecl> {
    declarations
        .into_iter()
        .filter(|decl| {
            let replacement = match decl.name.as_str() {
                "superagent.start" => "superagent_start",
                "superagent.stop" => "superagent_stop",
                "superagent.relay" => "superagent_relay",
                "superagent.set_mode" => "superagent_set_mode",
                _ => return true,
            };
            !registered(replacement)
        })
        .collect()
}

/// Normalises a prompt snippet.
fn normalize_snippet(text: Option<&str>) -> Option<String> {
    let text = text?;
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then_some(one_line)
}

/// Normalises prompt guidelines.
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

fn is_mcp(entry: &ToolEntry) -> bool {
    matches!(&entry.source, ToolSource::Extension(id) if id == "mcp" || id.ends_with("/mcp") || id.rsplit_once('.').is_some_and(|(_, suffix)| suffix == "mcp"))
        || entry.name().starts_with("mcp_")
}

fn patch_eligible(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("gpt")
        || model.contains("codex")
        || model
            .split(['/', '-'])
            .any(|part| matches!(part, "o1" | "o3" | "o4"))
}

/// A declaration's bytes never change, even if its live implementation disappears.
struct DeclaredTool {
    meta: oino_agent::ToolDefinitionMeta,
    inner: std::sync::Weak<super::Inner>,
}

impl AgentTool for DeclaredTool {
    fn meta(&self) -> &oino_agent::ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: oino_ai::JsonObject) -> oino_ai::JsonObject {
        self.inner
            .upgrade()
            .and_then(|inner| {
                lock(&inner.tools)
                    .registry
                    .get(&self.meta.name)
                    .map(|e| Arc::clone(&e.tool))
            })
            .map_or_else(|| raw.clone(), |tool| tool.prepare_arguments(raw.clone()))
    }

    fn execute(
        &self,
        call: oino_agent::ToolInvocation,
        updates: oino_agent::UpdateSink,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<
        'static,
        Result<oino_agent::AgentToolResult, oino_agent::ToolError>,
    > {
        let live = self.inner.upgrade().and_then(|inner| {
            let session = AgentSession::from_inner(Arc::clone(&inner));
            if self.meta.name == "apply_patch"
                && !patch_eligible(session.model().as_ref().map_or("", |m| m.id.as_str()))
            {
                return None;
            }
            lock(&inner.tools)
                .registry
                .get(&self.meta.name)
                .map(|e| Arc::clone(&e.tool))
        });
        let name = self.meta.name.clone();
        Box::pin(async move {
            match live {
                Some(tool) => tool.execute(call, updates, cancel).await,
                None => Err(oino_agent::ToolError::new(format!(
                    "tool {name} is unavailable"
                ))),
            }
        })
    }
}

fn js_identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn schema_typescript(schema: &JsonValue) -> String {
    if let Some(values) = schema.get("enum").and_then(JsonValue::as_array) {
        return values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    match schema.get("type").and_then(JsonValue::as_str) {
        Some("string") => "string".into(),
        Some("integer" | "number") => "number".into(),
        Some("boolean") => "boolean".into(),
        Some("null") => "null".into(),
        Some("array") => format!("({})[]", schema_typescript(&schema["items"])),
        Some("object") => {
            let fields = schema
                .get("properties")
                .and_then(JsonValue::as_object)
                .map(|properties| {
                    properties
                        .iter()
                        .map(|(key, value)| {
                            format!(
                                "{}{}: {}",
                                serde_json::json!(key),
                                if schema
                                    .get("required")
                                    .and_then(JsonValue::as_array)
                                    .is_some_and(|r| r.contains(&serde_json::json!(key)))
                                {
                                    ""
                                } else {
                                    "?"
                                },
                                schema_typescript(value)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            format!("{{ {fields} }}")
        }
        _ => "unknown".into(),
    }
}

fn append_codemode_declarations(
    meta: &mut oino_agent::ToolDefinitionMeta,
    state: &ToolState,
    mut remaining: usize,
) {
    for tool in state.registry.values().filter(|tool| {
        tool.name() != "codemode"
            && tool.exposure != ToolExposure::Hidden
            && tool.exposure != ToolExposure::ModelOnly
            && tool.exposure != ToolExposure::Deferred
            && !is_mcp(tool)
            && (state.codemode_only || tool.exposure == ToolExposure::Codemode)
    }) {
        let declaration = format!(
            "\n{}\ndeclare function {}(args: {}): Promise<unknown>;\n{}\n",
            tool.tool.meta().description,
            js_identifier(tool.name()),
            schema_typescript(&tool.tool.meta().parameters),
            tool.prompt_guidelines.join("\n")
        );
        if declaration.len() <= remaining {
            remaining -= declaration.len();
            meta.description.push_str(&declaration);
        }
    }
}

#[cfg(test)]
mod declaration_migration_tests {
    use super::migrate_superagent_declarations;
    use oino_ai::ToolDecl;
    use serde_json::json;

    #[test]
    fn stale_superagent_aliases_are_dropped_only_when_replacement_registered() {
        let decl = |name: &str| ToolDecl {
            name: name.into(),
            description: "old".into(),
            parameters: json!({}),
            constrained_sampling: None,
        };
        let tools = migrate_superagent_declarations(
            vec![
                decl("superagent.start"),
                decl("superagent.relay"),
                decl("other.tool"),
            ],
            |name| name == "superagent_start",
        );
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["superagent.relay", "other.tool"]
        );
    }
}
