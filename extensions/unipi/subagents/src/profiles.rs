//! Agent profiles: built-ins plus custom Markdown agents (unipi `profiles.ts`, `agents.ts`).
//!
//! Custom agents: `<name>.md` or `<name>/{AGENT,AGENTS,agent,agents}.md` with YAML front matter
//! (`name`, `description`, `model`, `tools`/`allowed-tools`, `thinking`, `max-nesting`); the body
//! is the appended system prompt. Global agents live in `/ext-global/profiles/`, project
//! agents in `.oino/agents/` (project wins; unipi-port §3.11).

use std::collections::BTreeMap;

/// Where a profile comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Built in.
    Builtin,
    /// `/ext-global/profiles`.
    Global,
    /// `.oino/agents`.
    Project,
}

impl Source {
    /// unipi label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// unipi `AgentProfile`.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Id.
    pub id: String,
    /// One-line description.
    pub description: String,
    /// Tool allowlist (`None` = all except the nesting tools).
    pub tools: Option<Vec<String>>,
    /// Model override.
    pub model: Option<String>,
    /// Thinking override.
    pub thinking: Option<String>,
    /// Nesting levels this agent may spawn.
    pub max_nesting: Option<u8>,
    /// Appended system prompt.
    pub system_prompt: String,
    /// Source.
    pub source: Source,
}

/// unipi `EXPLORE_TOOLS`.
pub const EXPLORE_TOOLS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "web_search",
    "memory_search",
    "memory_list",
];
/// unipi `EXCLUDED_NESTING_TOOLS`.
pub const EXCLUDED_NESTING_TOOLS: &[&str] = &["sidekick", "read_subagent", "run_subagent"];
/// Directory layouts searched for a custom agent.
pub const DIR_CANDIDATES: &[&str] = &["AGENT.md", "AGENTS.md", "agent.md", "agents.md"];

const EXPLORE_PROMPT: &str = "You are a read-only exploration subagent. Investigate the task using only read and search tools; you cannot edit files or run commands. Report concrete findings with file paths and line numbers, and say clearly what you could not determine.";
const GENERAL_PROMPT: &str = "You are a general-purpose subagent working on a task delegated by a parent agent. Complete the task, verify your work, and finish with a concise report: what you did, files changed, how you verified it, and anything left open. Do not talk to the user; your final message goes to the parent agent.";

/// unipi `builtinProfiles`.
#[must_use]
pub fn builtins() -> Vec<Profile> {
    vec![
        Profile {
            id: "subagent_explore".into(),
            description: "Read-only codebase exploration and research.".into(),
            tools: Some(EXPLORE_TOOLS.iter().map(|s| (*s).to_owned()).collect()),
            model: None,
            thinking: None,
            max_nesting: None,
            system_prompt: EXPLORE_PROMPT.into(),
            source: Source::Builtin,
        },
        Profile {
            id: "subagent_general".into(),
            description: "General-purpose tasks including code changes.".into(),
            tools: None,
            model: None,
            thinking: None,
            max_nesting: None,
            system_prompt: GENERAL_PROMPT.into(),
            source: Source::Builtin,
        },
    ]
}

/// A front-matter value.
#[derive(Clone, Debug, PartialEq)]
pub enum FmValue {
    /// Scalar text (quotes removed).
    Text(String),
    /// `- item` list.
    List(Vec<String>),
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return serde_json::from_str::<String>(t).unwrap_or_else(|_| t[1..t.len() - 1].to_owned());
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    t.to_owned()
}

/// pi `parseFrontmatter` subset: `---` block of `key: value`, `key: [a, b]` and `key:` + `- item`
/// lines; returns `(front matter, body)`.
#[must_use]
pub fn parse_front_matter(raw: &str) -> (BTreeMap<String, FmValue>, String) {
    let mut map = BTreeMap::new();
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (map, text.to_owned());
    };
    let (block, body) = match rest.find("\n---") {
        Some(i) => {
            let after = &rest[i + 4..];
            let body = after.split_once('\n').map_or("", |(_, b)| b);
            (&rest[..i], body)
        }
        None => return (map, text.to_owned()),
    };
    let mut current_list: Option<String> = None;
    for line in block.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed.trim_start().strip_prefix("- ")
            && let Some(key) = &current_list
        {
            if let Some(FmValue::List(items)) = map.get_mut(key) {
                items.push(unquote(item));
            }
            continue;
        }
        let Some((k, v)) = trimmed.split_once(':') else {
            continue;
        };
        let key = k.trim().to_owned();
        let value = v.trim();
        if value.is_empty() {
            map.insert(key.clone(), FmValue::List(Vec::new()));
            current_list = Some(key);
        } else if value.starts_with('[') && value.ends_with(']') {
            let items = value[1..value.len() - 1]
                .split(',')
                .map(unquote)
                .filter(|s| !s.is_empty())
                .collect();
            map.insert(key, FmValue::List(items));
            current_list = None;
        } else {
            map.insert(key, FmValue::Text(unquote(value)));
            current_list = None;
        }
    }
    (map, body.to_owned())
}

fn text<'a>(fm: &'a BTreeMap<String, FmValue>, key: &str) -> Option<&'a str> {
    match fm.get(key) {
        Some(FmValue::Text(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// unipi `parseToolList`.
fn tool_list(v: Option<&FmValue>) -> Option<Vec<String>> {
    match v? {
        FmValue::List(items) => Some(items.clone()),
        FmValue::Text(s) => Some(
            s.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        ),
    }
}

/// unipi `loadAgentFile`.
#[must_use]
pub fn parse_agent(raw: &str, source: Source, path_name: &str) -> Profile {
    let (fm, body) = parse_front_matter(raw);
    let id = text(&fm, "name")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(path_name)
        .to_owned();
    Profile {
        description: text(&fm, "description").map_or_else(|| id.clone(), ToOwned::to_owned),
        model: text(&fm, "model").map(ToOwned::to_owned),
        thinking: text(&fm, "thinking").map(ToOwned::to_owned),
        tools: tool_list(fm.get("allowed-tools").or_else(|| fm.get("tools"))),
        max_nesting: text(&fm, "max-nesting").and_then(|n| n.parse().ok()),
        system_prompt: body.trim().to_owned(),
        source,
        id,
    }
}

/// One agent file found in a directory: `(file name for warnings, path name, contents)`.
pub type AgentFile = (String, String, String);

/// Merges built-ins, global and project agents (unipi `loadProfiles`): customs shadowing a
/// built-in are skipped with a warning; project agents replace global ones of the same id.
#[must_use]
pub fn merge(global: &[AgentFile], project: &[AgentFile]) -> (Vec<Profile>, Vec<String>) {
    let builtins = builtins();
    let mut warnings = Vec::new();
    let mut merged: Vec<Profile> = Vec::new();
    for (files, source) in [(global, Source::Global), (project, Source::Project)] {
        let mut sorted: Vec<&AgentFile> = files.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, path_name, raw) in sorted {
            let p = parse_agent(raw, source, path_name);
            if builtins.iter().any(|b| b.id == p.id) {
                warnings.push(format!(
                    "{}: custom agent shadows a built-in name — skipped",
                    p.id
                ));
                continue;
            }
            match merged.iter_mut().find(|m| m.id == p.id) {
                Some(slot) => *slot = p,
                None => merged.push(p),
            }
        }
    }
    let mut all = builtins;
    all.extend(merged);
    (all, warnings)
}

/// unipi `NAME_RE` + duplicates.
#[must_use]
pub fn validate_name(name: &str, existing: &[String]) -> Option<String> {
    let ok = name.len() <= 48
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !ok {
        return Some("Use lowercase letters, digits, - or _ (max 48).".into());
    }
    if builtins().iter().any(|b| b.id == name) {
        return Some(format!("{name} is a built-in profile name."));
    }
    if existing.iter().any(|e| e == name) {
        return Some(format!("An agent named {name} already exists."));
    }
    None
}

/// A new agent draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Model.
    pub model: Option<String>,
    /// Tools.
    pub tools: Option<Vec<String>>,
    /// Thinking.
    pub thinking: Option<String>,
    /// Prompt.
    pub prompt: String,
}

/// unipi `agentMarkdown`.
#[must_use]
pub fn agent_markdown(d: &Draft) -> String {
    let mut fm = vec![
        format!("name: {}", d.name),
        format!(
            "description: {}",
            serde_json::to_string(&d.description).unwrap_or_default()
        ),
    ];
    if let Some(m) = d.model.as_deref().filter(|m| !m.is_empty()) {
        fm.push(format!("model: {m}"));
    }
    if let Some(t) = d.thinking.as_deref().filter(|t| !t.is_empty()) {
        fm.push(format!("thinking: {t}"));
    }
    if let Some(tools) = d.tools.as_ref().filter(|t| !t.is_empty()) {
        fm.push("allowed-tools:".into());
        fm.extend(tools.iter().map(|t| format!("  - {t}")));
    }
    format!("---\n{}\n---\n\n{}\n", fm.join("\n"), d.prompt.trim())
}

/// unipi `promptTemplate`.
#[must_use]
pub fn prompt_template(name: &str, description: &str) -> String {
    format!(
        "You are the {name} subagent: {description}\n\nWork only on the task you are given. You do not see the parent conversation; everything you need is in the task.\n\nFinish with a concise report for the parent agent: what you found or did, with file paths and line numbers, and anything you could not determine."
    )
}

/// unipi `describe` (profile manager rows).
#[must_use]
pub fn describe(p: &Profile) -> String {
    let tools = match &p.tools {
        None => "all tools".to_owned(),
        Some(t) if t.len() <= 4 => t.join(", "),
        Some(t) => format!("{} tools", t.len()),
    };
    let model = p.model.clone().unwrap_or_else(|| {
        if p.id == "subagent_general" {
            "your model".into()
        } else {
            "default subagent model".into()
        }
    });
    format!(
        "{} · {} · {model} · {tools} — {}",
        p.id,
        p.source.as_str(),
        p.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, raw: &str) -> AgentFile {
        (format!("{name}.md"), name.into(), raw.into())
    }

    #[test]
    fn parses_front_matter_and_agents() {
        let raw = "---\nname: reviewer\ndescription: \"Reviews code\"\nmodel: anthropic/claude\nthinking: high\nallowed-tools:\n  - read\n  - grep\nmax-nesting: 1\n---\n\nBe strict.\n";
        let p = parse_agent(raw, Source::Global, "x");
        assert_eq!(p.id, "reviewer");
        assert_eq!(p.description, "Reviews code");
        assert_eq!(p.model.as_deref(), Some("anthropic/claude"));
        assert_eq!(p.tools, Some(vec!["read".into(), "grep".into()]));
        assert_eq!(p.max_nesting, Some(1));
        assert_eq!(p.system_prompt, "Be strict.");
        let p = parse_agent("---\ntools: read, ls\n---\nbody", Source::Project, "plain");
        assert_eq!((p.id.as_str(), p.description.as_str()), ("plain", "plain"));
        assert_eq!(p.tools, Some(vec!["read".into(), "ls".into()]));
        let p = parse_agent("no front matter", Source::Project, "n");
        assert_eq!(p.system_prompt, "no front matter");
        let (fm, _) = parse_front_matter("---\ntools: [a, \"b\"]\n---\n");
        assert_eq!(
            fm.get("tools"),
            Some(&FmValue::List(vec!["a".into(), "b".into()]))
        );
    }

    #[test]
    fn merges_with_shadow_and_project_override() {
        let global = vec![
            file("a", "---\ndescription: global a\n---\n"),
            file("subagent_explore", "x"),
        ];
        let project = vec![file("a", "---\ndescription: project a\n---\n")];
        let (all, warnings) = merge(&global, &project);
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].description, "project a");
        assert_eq!(all[2].source, Source::Project);
        assert_eq!(
            warnings,
            ["subagent_explore: custom agent shadows a built-in name — skipped"]
        );
    }

    #[test]
    fn names_markdown_and_descriptions() {
        assert!(validate_name("Bad", &[]).is_some());
        assert_eq!(
            validate_name("subagent_general", &[]).as_deref(),
            Some("subagent_general is a built-in profile name.")
        );
        assert!(validate_name("dup", &["dup".into()]).is_some());
        assert_eq!(validate_name("rev-1", &[]), None);
        let md = agent_markdown(&Draft {
            name: "rev".into(),
            description: "Re \"view\"".into(),
            model: Some("a/b".into()),
            tools: Some(vec!["read".into()]),
            thinking: None,
            prompt: " Do it ".into(),
        });
        assert_eq!(
            md,
            "---\nname: rev\ndescription: \"Re \\\"view\\\"\"\nmodel: a/b\nallowed-tools:\n  - read\n---\n\nDo it\n"
        );
        let back = parse_agent(&md, Source::Global, "rev");
        assert_eq!(back.description, "Re \"view\"");
        let b = builtins();
        assert_eq!(
            describe(&b[0]),
            "subagent_explore · builtin · default subagent model · 7 tools — Read-only codebase exploration and research."
        );
        assert!(describe(&b[1]).contains("your model · all tools"));
        assert!(prompt_template("rev", "reviews").starts_with("You are the rev subagent: reviews"));
    }
}
