//! Per-skill state resolution and the vault (unipi `src/registry.ts`, `src/vault.ts`), pure.
//!
//! Defaults: discovered skills are on and listed; vault skills are off until a scope turns
//! them on. Explicit states (project over global) override the defaults.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The slice of pi's `Skill` the registry needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkill {
    /// Name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// `SKILL.md` path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// Directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}

impl CatalogSkill {
    /// From a pi `Skill` JSON value.
    #[must_use]
    pub fn from_value(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

/// Where a skill comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillSource {
    /// The skill vault.
    Vault,
    /// Inside the project.
    Project,
    /// The user's skill dirs.
    User,
    /// Bundled with unipi.
    Unipi,
    /// Anything else.
    Package,
}

impl SkillSource {
    /// Label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vault => "vault",
            Self::Project => "project",
            Self::User => "user",
            Self::Unipi => "unipi",
            Self::Package => "package",
        }
    }
}

/// Directory of a skill.
#[must_use]
pub fn skill_dir(s: &CatalogSkill) -> String {
    s.base_dir.clone().unwrap_or_else(|| {
        s.file_path
            .as_deref()
            .map(|f| f.strip_suffix("/SKILL.md").unwrap_or(f).to_owned())
            .unwrap_or_default()
    })
}

/// `path` is `dir` or under it.
#[must_use]
pub fn is_under_dir(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return false;
    }
    let d = if dir.ends_with('/') {
        dir.to_owned()
    } else {
        format!("{dir}/")
    };
    path == dir || path.starts_with(&d)
}

/// unipi's own bundled skills (`@pi-unipi`, a dev checkout, or the Oino bundle
/// `~/.oino/bundled/unipi.*`).
#[must_use]
pub fn is_bundled_skill_location(location: &str) -> bool {
    location.contains("/@pi-unipi/")
        || location.contains("/unipi/packages/")
        || location.contains("/bundled/unipi.")
}

/// Host locations the source classification needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// Project cwd.
    pub cwd: String,
    /// Vault dir (`~/.oino/skill-vault`).
    pub vault: String,
    /// Home dir, when known.
    pub home: Option<String>,
    /// Oino agent dir (`~/.oino`), when known.
    pub agent_dir: Option<String>,
}

/// Classifies a skill's source.
#[must_use]
pub fn skill_source(s: &CatalogSkill, p: &Places) -> SkillSource {
    let dir = skill_dir(s);
    if is_under_dir(&dir, &p.vault) {
        return SkillSource::Vault;
    }
    if is_bundled_skill_location(&dir) {
        return SkillSource::Unipi;
    }
    if is_under_dir(&dir, &p.cwd) {
        return SkillSource::Project;
    }
    let user_dirs = [
        p.home.as_ref().map(|h| format!("{h}/.agents/skills")),
        p.home.as_ref().map(|h| format!("{h}/.pi/agent/skills")),
        p.agent_dir.as_ref().map(|a| format!("{a}/skills")),
    ];
    if user_dirs.iter().flatten().any(|u| is_under_dir(&dir, u)) {
        return SkillSource::User;
    }
    SkillSource::Package
}

/// Bundled skills listed by default.
pub const DEFAULT_MUST_SHOW: &[&str] = &["summarize"];

/// Default "Must show" of a skill.
#[must_use]
pub fn default_must_show(name: &str, source: SkillSource) -> bool {
    source == SkillSource::Unipi && DEFAULT_MUST_SHOW.contains(&name)
}

/// Per-skill state as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    /// `false` = removed from the session (catalog and `/skill:name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Always listed and enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_show: Option<bool>,
}

/// Effective state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effective {
    /// Enabled.
    pub enabled: bool,
    /// Must show.
    pub must_show: bool,
}

/// unipi `effectiveState`.
#[must_use]
pub fn effective_state(state: Option<&SkillState>, source: SkillSource, name: &str) -> Effective {
    let must_show = state
        .and_then(|s| s.must_show)
        .unwrap_or_else(|| default_must_show(name, source));
    Effective {
        enabled: must_show
            || state
                .and_then(|s| s.enabled)
                .unwrap_or(source != SkillSource::Vault),
        must_show,
    }
}

/// Registry result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Pinned names.
    pub must_show: BTreeSet<String>,
    /// Listed skills (catalog order, by index).
    pub listed: Vec<usize>,
    /// Removed names.
    pub disabled: BTreeSet<String>,
}

/// unipi `applyRegistry`.
#[must_use]
pub fn apply_registry(
    catalog: &[CatalogSkill],
    states: &BTreeMap<String, SkillState>,
    p: &Places,
) -> Applied {
    let mut out = Applied::default();
    for (i, s) in catalog.iter().enumerate() {
        let eff = effective_state(states.get(&s.name), skill_source(s, p), &s.name);
        if eff.must_show {
            out.must_show.insert(s.name.clone());
        }
        if eff.enabled {
            out.listed.push(i);
        } else {
            out.disabled.insert(s.name.clone());
        }
    }
    out
}

/// `/skill:name args` → name.
#[must_use]
pub fn skill_command_name(text: &str) -> Option<String> {
    let t = text.trim();
    if !t.get(..7)?.eq_ignore_ascii_case("/skill:") {
        return None;
    }
    let tail = &t[7..];
    let first = tail.chars().next().filter(char::is_ascii_alphanumeric)?;
    let mut name = String::from(first);
    name.extend(
        tail[first.len_utf8()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-'),
    );
    Some(name)
}

/// Normalises stored states (legacy `"on" | "unlisted" | "off"`, dropped `discoverable`).
#[must_use]
pub fn normalize_states(raw: &Value) -> BTreeMap<String, SkillState> {
    let mut out = BTreeMap::new();
    let Some(obj) = raw.as_object() else {
        return out;
    };
    for (name, v) in obj {
        match v {
            Value::String(s) if s == "off" || s == "unlisted" => {
                out.insert(
                    name.clone(),
                    SkillState {
                        enabled: Some(false),
                        must_show: None,
                    },
                );
            }
            Value::String(s) if s == "on" => {
                out.insert(
                    name.clone(),
                    SkillState {
                        enabled: Some(true),
                        must_show: None,
                    },
                );
            }
            Value::Object(o) => {
                let mut st = SkillState {
                    enabled: o.get("enabled").and_then(Value::as_bool),
                    must_show: o.get("mustShow").and_then(Value::as_bool),
                };
                if o.get("discoverable") == Some(&Value::Bool(false))
                    && st.enabled != Some(false)
                    && st.must_show != Some(true)
                {
                    st.enabled = Some(false);
                }
                out.insert(name.clone(), st);
            }
            _ => {}
        }
    }
    out
}

/// Minimal front-matter reader: `name:` and `description:` (plain, `>`, `|`).
#[must_use]
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, None);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, None);
    };
    let lines: Vec<&str> = rest[..end].lines().collect();
    let (mut name, mut desc) = (None, None);
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let (key, val) = if let Some(v) = line.strip_prefix("name:") {
            ("name", v)
        } else if let Some(v) = line.strip_prefix("description:") {
            ("description", v)
        } else {
            i += 1;
            continue;
        };
        let mut value = val.trim().to_owned();
        if matches!(value.as_str(), ">" | "|" | ">-" | ">+" | "|-" | "|+") {
            let mut block = Vec::new();
            while i + 1 < lines.len()
                && (lines[i + 1].is_empty()
                    || (lines[i + 1].starts_with(char::is_whitespace)
                        && !lines[i + 1].trim().is_empty()))
            {
                i += 1;
                block.push(lines[i].trim());
            }
            block.join(" ").trim().clone_into(&mut value);
        }
        let value = value.strip_prefix(['"', '\'']).unwrap_or(&value).to_owned();
        let value = value.strip_suffix(['"', '\'']).unwrap_or(&value).to_owned();
        if key == "name" {
            name = Some(value);
        } else {
            desc = Some(value);
        }
        i += 1;
    }
    (name, desc)
}

/// Lists vault skills: directories with `SKILL.md`, up to three levels deep. `guest` is the
/// vault's guest path, `host` the host path reported in `filePath`/`baseDir`.
#[must_use]
pub fn list_vault_skills(guest: &str, host: &str) -> Vec<CatalogSkill> {
    fn walk(g: &str, h: &str, depth: u32, root: bool, out: &mut Vec<CatalogSkill>) {
        if depth > 3 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(g) else {
            return;
        };
        let mut names: Vec<(String, bool)> = rd
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    e.file_type().is_ok_and(|t| t.is_dir()),
                )
            })
            .collect();
        names.sort();
        if !root && names.iter().any(|(n, _)| n == "SKILL.md") {
            if let Ok(text) = std::fs::read_to_string(format!("{g}/SKILL.md")) {
                let (name, desc) = parse_frontmatter(&text);
                let name = name
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| h.rsplit('/').next().unwrap_or_default().to_owned());
                if let Some(d) = desc.filter(|d| !d.is_empty()) {
                    out.push(CatalogSkill {
                        name,
                        description: d,
                        file_path: Some(format!("{h}/SKILL.md")),
                        base_dir: Some(h.to_owned()),
                    });
                }
            }
            return;
        }
        for (n, is_dir) in names {
            if n.starts_with('.') || !is_dir {
                continue;
            }
            walk(
                &format!("{g}/{n}"),
                &format!("{h}/{n}"),
                depth + 1,
                false,
                out,
            );
        }
    }
    let mut out = Vec::new();
    walk(guest, host, 0, true, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use serde_json::json;

    use super::*;

    fn places() -> Places {
        Places {
            cwd: "/repo".into(),
            vault: "/home/u/.oino/skill-vault".into(),
            home: Some("/home/u".into()),
            agent_dir: Some("/home/u/.oino".into()),
        }
    }

    fn sk(name: &str, dir: &str) -> CatalogSkill {
        CatalogSkill {
            name: name.into(),
            description: "d".into(),
            file_path: None,
            base_dir: Some(dir.into()),
        }
    }

    fn st(enabled: Option<bool>, must_show: Option<bool>) -> SkillState {
        SkillState { enabled, must_show }
    }

    #[test]
    fn registry_defaults_and_states() {
        let cat = [
            sk("vaulted", "/home/u/.oino/skill-vault/vaulted"),
            sk("global-one", "/home/u/.agents/skills/global-one"),
            sk("quiet", "/home/u/.agents/skills/quiet"),
        ];
        let names = |a: &Applied| -> Vec<String> {
            a.listed.iter().map(|i| cat[*i].name.clone()).collect()
        };
        let r = apply_registry(
            &cat,
            &BTreeMap::from([("quiet".into(), st(Some(false), None))]),
            &places(),
        );
        assert_eq!(names(&r), ["global-one"]);
        assert_eq!(
            r.disabled.iter().cloned().collect::<Vec<_>>(),
            ["quiet", "vaulted"]
        );
        let on = apply_registry(
            &cat,
            &BTreeMap::from([
                ("vaulted".into(), st(Some(true), Some(true))),
                ("global-one".into(), st(Some(false), None)),
                ("quiet".into(), st(Some(false), Some(true))),
            ]),
            &places(),
        );
        assert_eq!(names(&on), ["vaulted", "quiet"]);
        assert_eq!(on.disabled.iter().collect::<Vec<_>>(), ["global-one"]);
        assert_eq!(
            on.must_show.iter().collect::<Vec<_>>(),
            ["quiet", "vaulted"]
        );
    }

    #[test]
    fn defaults_per_source() {
        let e = |s: Option<SkillState>, src, n| effective_state(s.as_ref(), src, n);
        let eff = |enabled, must_show| Effective { enabled, must_show };
        assert_eq!(e(None, SkillSource::Vault, ""), eff(false, false));
        assert_eq!(
            e(Some(st(Some(false), None)), SkillSource::User, ""),
            eff(false, false)
        );
        assert_eq!(e(None, SkillSource::User, ""), eff(true, false));
        assert_eq!(
            e(Some(st(None, Some(true))), SkillSource::Vault, ""),
            eff(true, true)
        );
        assert_eq!(e(None, SkillSource::Unipi, "summarize"), eff(true, true));
        assert_eq!(
            e(Some(st(None, Some(false))), SkillSource::Unipi, "summarize"),
            eff(true, false)
        );
        assert_eq!(e(None, SkillSource::User, "summarize"), eff(true, false));
        assert_eq!(
            skill_source(
                &sk("x", "/home/u/.oino/bundled/unipi.utility@3.0.0-ab/skills/x"),
                &places()
            ),
            SkillSource::Unipi
        );
        assert_eq!(
            skill_source(&sk("x", "/repo/.agents/skills/x"), &places()),
            SkillSource::Project
        );
        assert_eq!(
            skill_source(&sk("x", "/home/u/.oino/skills/x"), &places()),
            SkillSource::User
        );
        assert_eq!(
            skill_source(&sk("x", "/opt/x"), &places()),
            SkillSource::Package
        );
    }

    #[test]
    fn commands_and_states() {
        assert_eq!(
            skill_command_name("/skill:coffee-sandbox do it").as_deref(),
            Some("coffee-sandbox")
        );
        assert_eq!(skill_command_name("hello"), None);
        assert_eq!(skill_command_name("/SKILL:Ab"), Some("Ab".into()));
        assert_eq!(
            normalize_states(&json!({"quiet": {"discoverable": false}})),
            BTreeMap::from([("quiet".into(), st(Some(false), None))])
        );
        assert_eq!(
            normalize_states(&json!({"quiet": "unlisted"})),
            BTreeMap::from([("quiet".into(), st(Some(false), None))])
        );
        assert_eq!(
            normalize_states(&json!({"quiet": {"discoverable": false, "mustShow": true}})),
            BTreeMap::from([("quiet".into(), st(None, Some(true)))])
        );
        assert_eq!(
            normalize_states(
                &json!({"a": "off", "b": "unlisted", "c": {"mustShow": true, "x": 1}, "e": 3})
            ),
            BTreeMap::from([
                ("a".into(), st(Some(false), None)),
                ("b".into(), st(Some(false), None)),
                ("c".into(), st(None, Some(true))),
            ])
        );
        let r = apply_registry(
            &[sk("vaulted", "/home/u/.oino/skill-vault/vaulted")],
            &BTreeMap::new(),
            &places(),
        );
        assert!(r.listed.is_empty() && r.must_show.is_empty());
    }

    #[test]
    fn vault_listing() {
        let base = std::env::temp_dir().join(format!("unipi-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let deploy = base.join("pack/deploy-aws");
        std::fs::create_dir_all(&deploy).unwrap_or_default();
        std::fs::write(
            deploy.join("SKILL.md"),
            "---\nname: deploy-aws\ndescription: >-\n  Deploy to AWS.\n  Use for releases.\n---\n# x\n",
        )
        .unwrap_or_default();
        std::fs::create_dir_all(base.join("no-desc")).unwrap_or_default();
        std::fs::write(base.join("no-desc/SKILL.md"), "---\nname: no-desc\n---\n")
            .unwrap_or_default();
        let g = base.display().to_string();
        let list = list_vault_skills(&g, "/host/vault");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "deploy-aws");
        assert_eq!(list[0].description, "Deploy to AWS. Use for releases.");
        assert_eq!(
            list[0].base_dir.as_deref(),
            Some("/host/vault/pack/deploy-aws")
        );
        assert_eq!(
            parse_frontmatter("---\nname: a\ndescription: \"quoted\"\n---"),
            (Some("a".into()), Some("quoted".into()))
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
