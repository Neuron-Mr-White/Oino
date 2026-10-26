//! Structured system prompt (pi `core/system-prompt.ts`) with the D13 product substitutions of
//! runtime §9.4.
//!
//! Sections are ordered: `preamble` (untagged), then `tools`, `rules`, `docs` (default prompt
//! only), `addendum`, `project_context`, `skills`, `cwd`, then custom sections — every non-preamble
//! section wrapped as `<name>\n…\n</name>`.

use std::path::Path;

use indexmap::IndexMap;
use oino_ai::{Sections, SystemMessage, TimestampMs, UserTextField};
use serde::{Deserialize, Serialize};

use crate::context_files::ContextFile;
use crate::skills::{Skill, SkillReader, format_skills_for_prompt};

/// Default preamble (pi text with the product name substituted, D13).
pub const DEFAULT_PREAMBLE: &str = "You are an expert coding assistant operating inside Oino, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.";

/// Tool guideline exported by the bash tool family (pi text with D12/D13 substitution).
pub const ENV_GUIDELINE: &str = "You can inspect OINO_* environment variables (also exported as PI_*) for current model and session details.";

/// The pi guideline [`ENV_GUIDELINE`] replaces.
pub const PI_ENV_GUIDELINE: &str =
    "You can inspect PI_* environment variables for current model and session details.";

const TOOLS_FOOTER: &str = "In addition to the tools above, you may have access to other custom tools depending on the project.";

/// Options of one prompt build (pi `NormalizedBuildSystemPromptOptions`; camelCase on the wire,
/// sent to extensions as `before_agent_start.systemPromptOptions` and accepted back mutated).
/// Absent collections deserialise to pi's normalised defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSystemPromptOptions {
    /// Custom prompt replacing the default preamble, tools, rules and docs (SYSTEM.md / CLI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_prompt: Option<String>,
    /// Exact full replacement set by a `before_agent_start` handler (not recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_system_prompt: Option<String>,
    /// Active tools (default `read, bash, edit, write`).
    #[serde(default = "default_selected_tools")]
    pub selected_tools: Vec<String>,
    /// Selected tools whose declarations are hidden (`prepareLoadout`).
    #[serde(default)]
    pub hidden_tools: Vec<String>,
    /// One-line snippet per tool; tools without one are not listed.
    #[serde(default)]
    pub tool_snippets: IndexMap<String, String>,
    /// Guideline bullets per tool.
    #[serde(default)]
    pub tool_guidelines: IndexMap<String, Vec<String>>,
    /// Extra guideline bullets.
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
    /// Addendum (`APPEND_SYSTEM.md` / CLI, joined by `\n\n`).
    #[serde(default)]
    pub append_system_prompt: String,
    /// Custom sections keyed by tag name (`^[a-z][a-z0-9_-]*$`, not `preamble`).
    #[serde(default)]
    pub sections: IndexMap<String, String>,
    /// Working directory as shown to the model.
    #[serde(default)]
    pub cwd: String,
    /// Context files.
    #[serde(default)]
    pub context_files: Vec<ContextFile>,
    /// Loaded skills.
    #[serde(default)]
    pub skills: Vec<Skill>,
}

fn default_selected_tools() -> Vec<String> {
    oino_settings::tools::DEFAULT_TOOL_NAMES
        .iter()
        .map(|&name| name.to_owned())
        .collect()
}

impl Default for BuildSystemPromptOptions {
    fn default() -> Self {
        Self {
            custom_prompt: None,
            force_system_prompt: None,
            selected_tools: default_selected_tools(),
            hidden_tools: Vec::new(),
            tool_snippets: IndexMap::new(),
            tool_guidelines: IndexMap::new(),
            prompt_guidelines: Vec::new(),
            append_system_prompt: String::new(),
            sections: IndexMap::new(),
            cwd: String::new(),
            context_files: Vec::new(),
            skills: Vec::new(),
        }
    }
}

/// A custom section name pi rejects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Invalid system prompt section name: {name}")]
pub struct InvalidSectionName {
    /// Rejected name.
    pub name: String,
}

/// Prompt state of one build (pi `buildSystemPromptState`): a forced prompt is opaque text with
/// no sections; otherwise `content` is empty and `sections` carry the prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemPromptState {
    /// Forced prompt text, or empty.
    pub content: String,
    /// Structured sections (absent for a forced prompt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<IndexMap<String, String>>,
}

impl SystemPromptState {
    /// The state as a transcript system message (no tool changes).
    #[must_use]
    pub fn to_system_message(&self, timestamp: TimestampMs) -> SystemMessage {
        SystemMessage {
            content: UserTextField::Text(self.content.clone()),
            sections: self.sections.as_ref().map(|sections| {
                sections
                    .iter()
                    .map(|(name, text)| (name.clone(), Some(text.clone())))
                    .collect::<Sections>()
            }),
            tools_added: None,
            tools_removed: None,
            timestamp,
        }
    }

    /// Rendered text exactly as the transcript replays it (pi `getSystemMessageText`).
    #[must_use]
    pub fn render(&self) -> String {
        oino_ai::transcript::system_message_text(&self.to_system_message(TimestampMs::default()))
    }
}

/// The Oino docs section body with `{docs_root}` filled in (runtime §9.4).
#[must_use]
pub fn docs_section(docs_root: &Path) -> String {
    let root = docs_root.display();
    format!(
        "Oino documentation (read only when the user asks about Oino itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: {root}/README.md
- Additional docs: {root}/docs
- Examples: {root}/examples (extensions, custom tools, SDK)
- When reading Oino docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), extension API and WIT (docs/extension-api.md), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), providers and models (docs/providers.md, docs/models.md), Oino packages (docs/packages.md), environment variables (docs/environment-variables.md), RPC and ACP (docs/rpc.md, docs/acp.md)
- When working on Oino topics, read the docs and examples, and follow .md cross-references before implementing
- Always read Oino .md files completely and follow links to related docs (e.g., tui.md for TUI details)"
    )
}

/// pi `renderProjectContext`.
#[must_use]
pub fn render_project_context(files: &[ContextFile]) -> String {
    std::iter::once("Project-specific instructions and guidelines:".to_owned())
        .chain(files.iter().map(|file| {
            format!(
                "<project_instructions path=\"{}\">\n{}\n</project_instructions>",
                file.path.display(),
                file.content
            )
        }))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// pi `buildRules`: deduplicated, trimmed bullets.
#[must_use]
pub fn build_rules(
    declared_tools: &[String],
    tool_guidelines: &IndexMap<String, Vec<String>>,
    prompt_guidelines: &[String],
) -> String {
    let mut rules: Vec<String> = Vec::new();
    let mut add = |rule: &str| {
        let rule = rule.trim();
        if !rule.is_empty() && !rules.iter().any(|existing| existing == rule) {
            rules.push(rule.to_owned());
        }
    };
    let has = |name: &str| declared_tools.iter().any(|tool| tool == name);
    let (bash, powershell) = (has("bash"), has("powershell"));
    if (bash || powershell) && !has("grep") && !has("find") && !has("ls") {
        add(if bash && powershell {
            "Use bash or PowerShell for file operations like listing, searching, and finding files"
        } else if powershell {
            "Use PowerShell for file operations like listing, searching, and finding files"
        } else {
            "Use bash for file operations like ls, rg, find"
        });
    }
    for name in declared_tools {
        for rule in tool_guidelines.get(name).into_iter().flatten() {
            add(rule);
        }
    }
    for rule in prompt_guidelines {
        add(rule);
    }
    add("Be concise in your responses");
    add("Show file paths clearly when working with files");
    rules
        .iter()
        .map(|rule| format!("- {rule}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn valid_section_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// `preamble`, `tools`, `rules` and `docs` of the default prompt.
fn insert_default_sections(
    raw: &mut IndexMap<String, String>,
    options: &BuildSystemPromptOptions,
    declared: &[String],
    docs_root: &Path,
) {
    raw.insert("preamble".into(), DEFAULT_PREAMBLE.to_owned());
    let visible: Vec<String> = declared
        .iter()
        .filter_map(|name| {
            options
                .tool_snippets
                .get(name)
                .filter(|snippet| !snippet.is_empty())
                .map(|snippet| format!("- {name}: {snippet}"))
        })
        .collect();
    let tools = if visible.is_empty() {
        "(none)".to_owned()
    } else {
        visible.join("\n")
    };
    raw.insert("tools".into(), format!("{tools}\n\n{TOOLS_FOOTER}"));
    raw.insert(
        "rules".into(),
        build_rules(
            declared,
            &options.tool_guidelines,
            &options.prompt_guidelines,
        ),
    );
    raw.insert("docs".into(), docs_section(docs_root));
}

/// pi `buildSystemPromptSections` (ignores `force_system_prompt`).
///
/// # Errors
///
/// [`InvalidSectionName`] for a custom section named `preamble` or not matching
/// `^[a-z][a-z0-9_-]*$`.
pub fn build_system_prompt_sections(
    options: &BuildSystemPromptOptions,
    docs_root: &Path,
) -> Result<IndexMap<String, String>, InvalidSectionName> {
    if let Some(name) = options
        .sections
        .keys()
        .find(|name| !valid_section_name(name) || name.as_str() == "preamble")
    {
        return Err(InvalidSectionName { name: name.clone() });
    }
    let declared: Vec<String> = options
        .selected_tools
        .iter()
        .filter(|name| !options.hidden_tools.contains(name))
        .cloned()
        .collect();
    let mut raw: IndexMap<String, String> = IndexMap::new();
    if let Some(custom) = options
        .custom_prompt
        .as_deref()
        .filter(|prompt| !prompt.is_empty())
    {
        raw.insert("preamble".into(), custom.to_owned());
    } else {
        insert_default_sections(&mut raw, options, &declared, docs_root);
    }
    if !options.append_system_prompt.is_empty() {
        raw.insert("addendum".into(), options.append_system_prompt.clone());
    }
    if !options.context_files.is_empty() {
        raw.insert(
            "project_context".into(),
            render_project_context(&options.context_files),
        );
    }
    let reader = ["read", "bash"]
        .into_iter()
        .find(|tool| declared.iter().any(|name| name == tool))
        .map(|tool| {
            if tool == "read" {
                SkillReader::Read
            } else {
                SkillReader::Bash
            }
        })
        .or_else(|| {
            ["read", "bash"]
                .iter()
                .any(|tool| options.selected_tools.iter().any(|name| name == tool))
                .then_some(SkillReader::Indirect)
        });
    if let Some(reader) = reader
        && !options.skills.is_empty()
    {
        let skills = format_skills_for_prompt(&options.skills, reader);
        let skills = skills.trim();
        if !skills.is_empty() {
            raw.insert("skills".into(), skills.to_owned());
        }
    }
    raw.insert("cwd".into(), options.cwd.replace('\\', "/"));
    for (name, content) in &options.sections {
        if !content.is_empty() {
            raw.insert(name.clone(), content.clone());
        }
    }
    Ok(raw
        .into_iter()
        .map(|(name, content)| {
            if name == "preamble" {
                (name, content)
            } else {
                let wrapped = format!("<{name}>\n{content}\n</{name}>");
                (name, wrapped)
            }
        })
        .collect())
}

/// pi `buildSystemPromptState`.
///
/// # Errors
///
/// See [`build_system_prompt_sections`].
pub fn build_system_prompt_state(
    options: &BuildSystemPromptOptions,
    docs_root: &Path,
) -> Result<SystemPromptState, InvalidSectionName> {
    if let Some(forced) = &options.force_system_prompt {
        return Ok(SystemPromptState {
            content: forced.clone(),
            sections: None,
        });
    }
    Ok(SystemPromptState {
        content: String::new(),
        sections: Some(build_system_prompt_sections(options, docs_root)?),
    })
}

/// pi `buildSystemPrompt`: the rendered prompt text.
///
/// # Errors
///
/// See [`build_system_prompt_sections`].
pub fn build_system_prompt(
    options: &BuildSystemPromptOptions,
    docs_root: &Path,
) -> Result<String, InvalidSectionName> {
    build_system_prompt_state(options, docs_root).map(|state| state.render())
}

/// pi `diffSystemPromptSections`: the section patch from `previous` (replayed) to `current`, or
/// `None` when unchanged.
#[must_use]
pub fn diff_system_prompt_sections(
    previous: &IndexMap<String, String>,
    current: &IndexMap<String, String>,
) -> Option<Sections> {
    oino_ai::transcript::diff_sections(previous, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_section_names_are_rejected() {
        let mut options = BuildSystemPromptOptions::default();
        options.sections.insert("Bad".into(), "x".into());
        assert_eq!(
            build_system_prompt_sections(&options, Path::new("/d"))
                .unwrap_err()
                .to_string(),
            "Invalid system prompt section name: Bad"
        );
        options.sections = IndexMap::from([("preamble".into(), "x".into())]);
        assert!(build_system_prompt_sections(&options, Path::new("/d")).is_err());
    }

    #[test]
    fn deserialize_applies_pi_defaults() {
        let options: BuildSystemPromptOptions = serde_json::from_str(r#"{"cwd": "/w"}"#).unwrap();
        assert_eq!(options.selected_tools, ["read", "bash", "edit", "write"]);
        assert_eq!(options.cwd, "/w");
    }

    #[test]
    fn forced_prompt_has_no_sections() {
        let options = BuildSystemPromptOptions {
            force_system_prompt: Some("forced".into()),
            ..BuildSystemPromptOptions::default()
        };
        let state = build_system_prompt_state(&options, Path::new("/d")).unwrap();
        assert_eq!(state.sections, None);
        assert_eq!(state.render(), "forced");
    }
}
