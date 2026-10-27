//! The enchanted slash-command suggestions (unipi `src/provider.ts`), as a pure function from the
//! line before the cursor and the session's command list to an OUS completion group.
//!
//! unipi wraps pi's base autocomplete provider; Oino frontends merge `complete-input` groups
//! instead (ui-protocol §7.3), so this module first rebuilds pi's base slash-command list
//! (built-ins + `control.commands-list`, fuzzy-filtered exactly like pi-tui), then applies
//! unipi's enchantment and returns one group that **replaces** the frontend's own slash group
//! (`replaces: "commands"`, see deviations).

use serde::Serialize;
use serde_json::Value;

use crate::fuzzy::{fuzzy_filter, fuzzy_match};
use crate::registry::{
    self, BUILTIN_COMMANDS, COMMAND_DESCRIPTIONS, COMMAND_REGISTRY, PACKAGE_LABELS, PACKAGE_TONES,
};
use crate::sorting::{Tagged, sort_tagged};

/// One command the session knows (built-in, extension, prompt template or skill).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// Name without `/` (`plan`, `skill:x`, `compact`).
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Argument hint (`<level>`).
    pub argument_hint: Option<String>,
}

impl CommandInfo {
    /// From a pi `SlashCommandInfo` JSON value (`name`, `description?`, `argumentHint?`).
    #[must_use]
    pub fn from_json(v: &Value) -> Option<Self> {
        let name = v.get("name")?.as_str()?.to_owned();
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(ToOwned::to_owned);
        Some(Self {
            name,
            description: text("description"),
            argument_hint: text("argumentHint"),
        })
    }

    /// pi's built-in slash commands.
    #[must_use]
    pub fn builtins() -> Vec<Self> {
        BUILTIN_COMMANDS
            .iter()
            .map(|(name, desc, hint)| Self {
                name: (*name).to_owned(),
                description: Some((*desc).to_owned()),
                argument_hint: hint.map(ToOwned::to_owned),
            })
            .collect()
    }
}

/// One completion item (ui-protocol §7.3 `CompletionItem`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Inserted value (command name without `/`).
    pub value: String,
    /// Label.
    pub label: String,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Package tag (`[workflow]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Tag colour (OUS tone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Explicit rank (0 = first).
    pub sort_tier: u32,
}

/// The completion group returned from `complete-input`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Contributor (`unipi.command-enchantment`).
    pub source: String,
    /// The frontend's group this one supersedes (`commands`).
    pub replaces: String,
    /// Text the items complete (pi `prefix`, the trimmed slash text).
    pub prefix: String,
    /// Items in display order.
    pub items: Vec<Item>,
}

/// pi prefixes extension command descriptions with source tags (`[u:npm:@pi-unipi/unipi]`).
#[must_use]
pub fn strip_source_tag(description: &str) -> String {
    let d = description;
    if let Some(rest) = d.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let tag = &rest[..end];
        let mut chars = tag.chars();
        let ok = matches!(chars.next(), Some('u' | 'p' | 't'))
            && (tag.len() == 1 || tag.as_bytes().get(1) == Some(&b':') && tag.len() > 2);
        if ok {
            return rest[end + 1..].trim_start().to_owned();
        }
    }
    d.trim_start().to_owned()
}

/// unipi namespace aliases: a query naming a package floats its commands to the top.
#[must_use]
pub fn namespace_boost(query: &str) -> Option<&'static str> {
    const ALIASES: &[(&str, &str)] = &[
        ("workflow", "workflow"),
        ("ralph", "long-horizon"),
        ("goal", "long-horizon"),
        ("swarm", "long-horizon"),
        ("graph", "long-horizon"),
        ("lh", "long-horizon"),
        ("memory", "memory"),
        ("mcp", "mcp"),
        ("utility", "utility"),
        ("ask-user", "ask-user"),
        ("info", "info"),
        ("web-api", "web-api"),
        ("compact", "compact"),
        ("notify", "notify"),
        ("mem", "memory"),
        ("util", "utility"),
        ("web", "web-api"),
        ("notification", "notify"),
    ];
    if query.is_empty() {
        return None;
    }
    registry::lookup(ALIASES, &query.to_lowercase())
}

/// unipi `getEnhancedUnipiItems`: the feature commands for `prefix` (`/pl`, `/kanb`, …), boosted
/// package first, then exact > prefix > fuzzy, package order, name.
#[must_use]
pub fn enhanced_items(prefix: &str, overrides: &[(String, String)]) -> Vec<Item> {
    let query = prefix.strip_prefix('/').unwrap_or(prefix).to_lowercase();
    let entries: Vec<(String, &str)> = COMMAND_REGISTRY
        .iter()
        .map(|(c, p)| ((*c).to_owned(), *p))
        .collect();
    let boosted = namespace_boost(&query);
    let mut matched: Vec<(String, &str)> = if boosted.is_some() {
        entries
    } else {
        entries
            .into_iter()
            .filter(|(cmd, _)| fuzzy_match(&query, &cmd.to_lowercase()).matches)
            .collect()
    };
    let quality = |cmd: &str| -> u8 {
        let name = cmd.to_lowercase();
        if name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else {
            2
        }
    };
    matched.sort_by(|a, b| {
        let boost = |p: &str| u8::from(Some(p) != boosted);
        boost(a.1)
            .cmp(&boost(b.1))
            .then_with(|| quality(&a.0).cmp(&quality(&b.0)))
            .then_with(|| registry::package_rank(a.1).cmp(&registry::package_rank(b.1)))
            .then_with(|| a.0.cmp(&b.0))
    });
    matched
        .into_iter()
        .map(|(cmd, pkg)| {
            let label = registry::lookup(PACKAGE_LABELS, pkg).unwrap_or(pkg);
            let desc = overrides
                .iter()
                .find(|(c, _)| *c == cmd)
                .map(|(_, d)| d.clone())
                .or_else(|| registry::lookup(COMMAND_DESCRIPTIONS, &cmd).map(ToOwned::to_owned))
                .filter(|d| !d.is_empty());
            Item {
                label: cmd.clone(),
                value: cmd,
                description: desc,
                tag: Some(format!("[{label}]")),
                tone: registry::lookup(PACKAGE_TONES, pkg).map(ToOwned::to_owned),
                sort_tier: 0,
            }
        })
        .collect()
}

/// pi-tui base slash-command suggestions for `query` (bare-name matches first, then `skill:`
/// full-name-only matches); descriptions are `hint — description`.
#[must_use]
pub fn base_items(commands: &[CommandInfo], query: &str) -> Vec<Item> {
    let bare = fuzzy_filter(commands, query, |c| {
        c.name.strip_prefix("skill:").unwrap_or(&c.name)
    });
    let rest: Vec<CommandInfo> = commands
        .iter()
        .filter(|c| c.name.starts_with("skill:") && !bare.iter().any(|b| b.name == c.name))
        .cloned()
        .collect();
    let full = fuzzy_filter(&rest, query, |c| &c.name);
    bare.into_iter()
        .chain(full)
        .map(|c| {
            let desc = c.description.clone().unwrap_or_default();
            let full_desc = match &c.argument_hint {
                Some(h) if desc.is_empty() => h.clone(),
                Some(h) => format!("{h} — {desc}"),
                None => desc,
            };
            Item {
                value: c.name.clone(),
                label: c.name.clone(),
                description: (!full_desc.is_empty()).then_some(full_desc),
                tag: None,
                tone: None,
                sort_tier: 0,
            }
        })
        .collect()
}

/// Line text before the cursor for a `{text, cursor}` request (`cursor` in characters).
#[must_use]
pub fn line_before_cursor(text: &str, cursor: Option<usize>) -> String {
    let upto: String = match cursor {
        Some(c) => text.chars().take(c).collect(),
        None => text.to_owned(),
    };
    upto.rsplit('\n').next().unwrap_or("").to_owned()
}

fn group(prefix: &str, items: Vec<Item>) -> Group {
    Group {
        source: crate::EXTENSION_ID.to_owned(),
        replaces: "commands".to_owned(),
        prefix: prefix.to_owned(),
        items: items
            .into_iter()
            .zip(0_u32..)
            .map(|(mut item, i)| {
                item.sort_tier = i;
                item
            })
            .collect(),
    }
}

/// The enchanted suggestions for the line before the cursor; `None` = pass through (not a
/// command name, argument position, or nothing matches).
#[must_use]
pub fn suggest(line: &str, commands: &[CommandInfo]) -> Option<Group> {
    let command_text = line.trim_start();
    if !command_text.starts_with('/') || command_text.contains(' ') {
        // Argument completion stays with the host (`complete-command`).
        return None;
    }
    let prefix = command_text.to_owned();
    let base = base_items(commands, &command_text[1..]);
    let mut non_feature = Vec::new();
    let mut overrides = Vec::new();
    for item in base {
        if registry::package_of(&item.value).is_some() {
            if let Some(d) = &item.description {
                let clean = strip_source_tag(d);
                if !clean.is_empty() {
                    overrides.push((item.value.clone(), clean));
                }
            }
        } else {
            non_feature.push(item);
        }
    }
    let enhanced = enhanced_items(&prefix, &overrides);
    let stripped = prefix[1..].to_lowercase();
    let skill_query = stripped.starts_with("skill:");
    if enhanced.is_empty() {
        if non_feature.is_empty() {
            return None;
        }
        if skill_query {
            return Some(group(&prefix, non_feature));
        }
        let system: Vec<Item> = non_feature
            .into_iter()
            .filter(|i| !i.value.starts_with("skill:"))
            .collect();
        return (!system.is_empty()).then(|| group(&prefix, system));
    }
    let (skills, system): (Vec<Item>, Vec<Item>) = non_feature
        .into_iter()
        .partition(|i| i.value.starts_with("skill:"));
    let items = if skill_query {
        skills.into_iter().chain(enhanced).chain(system).collect()
    } else {
        let tagged: Vec<Tagged<Item>> = system
            .into_iter()
            .map(|i| (i, false))
            .chain(enhanced.into_iter().map(|i| (i, true)))
            .map(|(item, is_feature)| Tagged {
                value: item.value.clone(),
                item,
                is_feature,
            })
            .collect();
        sort_tagged(&tagged, &stripped)
            .into_iter()
            .map(|t| t.item)
            .collect()
    };
    Some(group(&prefix, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str, desc: Option<&str>) -> CommandInfo {
        CommandInfo {
            name: name.to_owned(),
            description: desc.map(ToOwned::to_owned),
            argument_hint: None,
        }
    }

    fn session() -> Vec<CommandInfo> {
        let mut c = CommandInfo::builtins();
        c.push(cmd("plan", Some("[u:npm:@pi-unipi/unipi] Live plan")));
        c.push(cmd("brand-new", Some("Fresh command")));
        c.push(cmd("skill:audit", Some("Audit skill")));
        c.push(cmd("review", Some("prompt")));
        c
    }

    fn values(g: &Group) -> Vec<&str> {
        g.items.iter().map(|i| i.value.as_str()).collect()
    }

    #[test]
    fn source_tags_are_stripped() {
        assert_eq!(strip_source_tag("[u:npm:@pi-unipi/unipi] Plan"), "Plan");
        assert_eq!(strip_source_tag("[p] Prompt"), "Prompt");
        assert_eq!(strip_source_tag("[workflow] keep"), "[workflow] keep");
        assert_eq!(strip_source_tag("  plain"), "plain");
    }

    #[test]
    fn not_a_command_or_argument_position_passes_through() {
        assert!(suggest("hello", &session()).is_none());
        assert!(suggest("/plan view", &session()).is_none());
        assert!(suggest("/zzzzqqq", &session()).is_none());
    }

    #[test]
    fn feature_items_get_tags_tones_and_overrides() {
        let g = suggest("/pl", &session()).unwrap_or_else(|| group("", vec![]));
        assert_eq!(g.replaces, "commands");
        assert_eq!(g.prefix, "/pl");
        let plan = g.items.iter().find(|i| i.value == "plan");
        let plan = plan.map(|p| {
            (
                p.tag.as_deref(),
                p.tone.as_deref(),
                p.description.as_deref(),
            )
        });
        assert_eq!(
            plan,
            Some((Some("[workflow]"), Some("c1"), Some("Live plan")))
        );
        assert_eq!(g.items[0].value, "plan");
        assert!(
            g.items
                .iter()
                .enumerate()
                .all(|(i, it)| it.sort_tier as usize == i)
        );
    }

    #[test]
    fn builtins_win_and_other_extension_commands_stay_untagged() {
        let g = suggest("/settings", &session()).unwrap_or_else(|| group("", vec![]));
        assert_eq!(values(&g).first(), Some(&"settings"));
        assert_eq!(g.items[0].tag, None);
        let g = suggest("/brand", &session()).unwrap_or_else(|| group("", vec![]));
        let item = g.items.iter().find(|i| i.value == "brand-new");
        assert_eq!(item.and_then(|i| i.tag.as_deref()), None);
        assert_eq!(
            item.and_then(|i| i.description.as_deref()),
            Some("Fresh command")
        );
    }

    #[test]
    fn namespace_queries_float_the_package() {
        let g = suggest("/mem", &session()).unwrap_or_else(|| group("", vec![]));
        let first = g
            .items
            .iter()
            .find(|i| i.tag.is_some())
            .map(|i| i.tag.clone());
        assert_eq!(first, Some(Some("[memory]".to_owned())));
        assert!(
            g.items.len() > 10,
            "namespace mode lists every feature command"
        );
    }

    #[test]
    fn skills_hidden_until_skill_prefix() {
        let g = suggest("/au", &session()).unwrap_or_else(|| group("", vec![]));
        assert!(values(&g).contains(&"kanboard-autowork"));
        assert!(
            !values(&g).contains(&"skill:audit"),
            "skills hidden next to feature matches"
        );
        let g = suggest("/skill:au", &session()).unwrap_or_else(|| group("", vec![]));
        assert_eq!(values(&g).first(), Some(&"skill:audit"));
        // No feature match and no explicit skill query: skills are filtered out.
        let only_skill = vec![cmd("skill:qwop", None), cmd("qwerty", None)];
        let g = suggest("/qw", &only_skill).unwrap_or_else(|| group("", vec![]));
        assert_eq!(values(&g), ["qwerty"]);
    }

    #[test]
    fn base_items_compose_argument_hints() {
        let items = base_items(&CommandInfo::builtins(), "thinking");
        assert_eq!(
            items[0].description.as_deref(),
            Some("<level> — Set thinking level")
        );
    }

    #[test]
    fn line_before_cursor_takes_the_current_line() {
        assert_eq!(line_before_cursor("a\n/kan", None), "/kan");
        assert_eq!(line_before_cursor("/plan rest", Some(4)), "/pla");
    }
}
