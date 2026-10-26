//! Skills (pi `core/skills.ts`): discovery, frontmatter validation, prompt formatting and
//! `/skill:name` expansion.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use oino_settings::paths::{canonicalize_or_self, is_under_path, resolve_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diagnostics::{CollisionResource, ResourceDiagnostic};
use crate::frontmatter::{parse_frontmatter, strip_frontmatter};
use crate::source::{SourceInfo, SourceScope};
use crate::walk::{IgnoreMatcher, has_suffix, posix_relative, read_dir_sorted};

/// Maximum skill name length (Agent Skills spec).
const MAX_NAME_LENGTH: usize = 64;
/// Maximum description length (Agent Skills spec).
const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// A loaded skill (pi `Skill`; serialised with pi field names).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// Frontmatter `name`, else the parent directory name.
    pub name: String,
    /// Frontmatter `description` (required, non-empty).
    pub description: String,
    /// Skill file (`SKILL.md` or a standalone `.md`).
    pub file_path: PathBuf,
    /// Directory relative references resolve against (the file's directory).
    pub base_dir: PathBuf,
    /// Provenance.
    pub source_info: SourceInfo,
    /// `disable-model-invocation: true`: hidden from the prompt, still invocable by command.
    pub disable_model_invocation: bool,
}

/// Skills plus diagnostics (pi `LoadSkillsResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadSkillsResult {
    /// Loaded skills (first-wins by name).
    pub skills: Vec<Skill>,
    /// Validation, read and collision diagnostics.
    pub diagnostics: Vec<ResourceDiagnostic>,
}

impl LoadSkillsResult {
    fn extend(&mut self, other: Self) {
        self.skills.extend(other.skills);
        self.diagnostics.extend(other.diagnostics);
    }
}

/// Source label used for skill provenance (pi `loadSkillsFromDir` `source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillSource {
    /// Under `<agent>/skills`.
    User,
    /// Under `<cwd>/.oino/skills`.
    Project,
    /// Any other path.
    Path,
}

/// Inputs of [`load_skills`] (pi `LoadSkillsOptions`).
#[derive(Clone, Debug)]
pub struct LoadSkillsOptions<'a> {
    /// Working directory (relative skill paths resolve against it).
    pub cwd: &'a Path,
    /// Agent dir.
    pub agent_dir: &'a Path,
    /// Home dir for `~` expansion.
    pub home: Option<&'a Path>,
    /// Explicit skill files or directories, in precedence order.
    pub skill_paths: &'a [PathBuf],
    /// Also scan `<agent>/skills` and `<cwd>/.oino/skills` first.
    pub include_defaults: bool,
}

/// pi `validateName`.
fn validate_name(name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let length = name.encode_utf16().count();
    if length > MAX_NAME_LENGTH {
        errors.push(format!(
            "name exceeds {MAX_NAME_LENGTH} characters ({length})"
        ));
    }
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        errors.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)"
                .to_owned(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name must not start or end with a hyphen".to_owned());
    }
    if name.contains("--") {
        errors.push("name must not contain consecutive hyphens".to_owned());
    }
    errors
}

/// pi `validateDescription`.
fn validate_description(description: Option<&Value>) -> Vec<String> {
    match description.and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => {
            let length = text.encode_utf16().count();
            if length > MAX_DESCRIPTION_LENGTH {
                vec![format!(
                    "description exceeds {MAX_DESCRIPTION_LENGTH} characters ({length})"
                )]
            } else {
                Vec::new()
            }
        }
        _ => vec!["description is required".to_owned()],
    }
}

fn skill_source_info(file_path: &Path, base_dir: &Path, source: SkillSource) -> SourceInfo {
    let base = Some(base_dir.to_path_buf());
    match source {
        SkillSource::User => {
            SourceInfo::synthetic(file_path, "local", Some(SourceScope::User), base)
        }
        SkillSource::Project => {
            SourceInfo::synthetic(file_path, "local", Some(SourceScope::Project), base)
        }
        SkillSource::Path => SourceInfo::synthetic(file_path, "local", None, base),
    }
}

/// pi `loadSkillFromFile`: one skill file, with diagnostics. Standalone `.md` files without a
/// description are silently skipped; `SKILL.md` files report every problem.
#[must_use]
pub fn load_skill_from_file(
    file_path: &Path,
    source: SkillSource,
) -> (Option<Skill>, Vec<ResourceDiagnostic>) {
    let mut diagnostics = Vec::new();
    let is_declared = file_path.file_name().is_some_and(|name| name == "SKILL.md");
    let raw = match fs::read_to_string(file_path) {
        Ok(raw) => raw,
        Err(error) => {
            diagnostics.push(ResourceDiagnostic::warning(error.to_string(), file_path));
            return (None, diagnostics);
        }
    };
    let frontmatter = match parse_frontmatter(&raw) {
        Ok(parsed) => parsed.fields,
        Err(error) => {
            if is_declared {
                diagnostics.push(ResourceDiagnostic::warning(error.message, file_path));
            }
            return (None, diagnostics);
        }
    };
    let description = frontmatter.get("description");
    let description_text = description
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    if !is_declared && description_text.is_none() {
        return (None, diagnostics);
    }
    let skill_dir = file_path.parent().unwrap_or(file_path).to_path_buf();
    let parent_name = skill_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    for error in validate_description(description) {
        diagnostics.push(ResourceDiagnostic::warning(error, file_path));
    }
    let name = frontmatter
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map_or(parent_name, str::to_owned);
    for error in validate_name(&name) {
        diagnostics.push(ResourceDiagnostic::warning(error, file_path));
    }
    let Some(description) = description_text else {
        return (None, diagnostics);
    };
    let skill = Skill {
        name,
        description: description.to_owned(),
        file_path: file_path.to_path_buf(),
        base_dir: skill_dir.clone(),
        source_info: skill_source_info(file_path, &skill_dir, source),
        disable_model_invocation: frontmatter.get("disable-model-invocation")
            == Some(&Value::Bool(true)),
    };
    (Some(skill), diagnostics)
}

/// pi `loadSkillsFromDir`: a dir with `SKILL.md` is one skill (no recursion); otherwise direct
/// `.md` children of the root are skills and subdirectories are searched for `SKILL.md`.
/// Dot entries, `node_modules` and ignore-file matches are skipped.
#[must_use]
pub fn load_skills_from_dir(dir: &Path, source: SkillSource) -> LoadSkillsResult {
    let mut ignore = IgnoreMatcher::new(dir);
    load_dir(dir, source, true, &mut ignore, dir)
}

fn load_dir(
    dir: &Path,
    source: SkillSource,
    include_root_files: bool,
    ignore: &mut IgnoreMatcher,
    root: &Path,
) -> LoadSkillsResult {
    let mut result = LoadSkillsResult::default();
    if !dir.exists() {
        return result;
    }
    ignore.add_rules(dir);
    let Some(entries) = read_dir_sorted(dir) else {
        return result;
    };
    if let Some(entry) = entries.iter().find(|entry| entry.name == "SKILL.md")
        && entry.is_file
        && !ignore.ignores(&posix_relative(root, &entry.path), false)
    {
        let (skill, diagnostics) = load_skill_from_file(&entry.path, source);
        result.skills.extend(skill);
        result.diagnostics.extend(diagnostics);
        return result;
    }
    for entry in &entries {
        if entry.name.starts_with('.') || entry.name == "node_modules" {
            continue;
        }
        if ignore.ignores(&posix_relative(root, &entry.path), entry.is_dir) {
            continue;
        }
        if entry.is_dir {
            result.extend(load_dir(&entry.path, source, false, ignore, root));
            continue;
        }
        if !entry.is_file || !include_root_files || !has_suffix(&entry.name, ".md") {
            continue;
        }
        let (skill, diagnostics) = load_skill_from_file(&entry.path, source);
        result.skills.extend(skill);
        result.diagnostics.extend(diagnostics);
    }
    result
}

/// pi `loadSkills`: load every location, first skill per name wins (collision diagnostics
/// last), the same real file is loaded once.
#[must_use]
pub fn load_skills(options: &LoadSkillsOptions<'_>) -> LoadSkillsResult {
    let user_dir = options.agent_dir.join("skills");
    let project_dir = oino_settings::paths::project_dir(options.cwd).join("skills");
    let mut collector = SkillCollector::default();

    if options.include_defaults {
        collector.add(load_skills_from_dir(&user_dir, SkillSource::User));
        collector.add(load_skills_from_dir(&project_dir, SkillSource::Project));
    }
    let source_for = |path: &Path| {
        if !options.include_defaults {
            if is_under_path(path, &user_dir) {
                return SkillSource::User;
            }
            if is_under_path(path, &project_dir) {
                return SkillSource::Project;
            }
        }
        SkillSource::Path
    };

    for raw in options.skill_paths {
        let path = resolve_path(&raw.to_string_lossy(), options.cwd, options.home, true);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                collector.diagnostics.push(ResourceDiagnostic::warning(
                    "skill path does not exist",
                    &path,
                ));
                continue;
            }
            Err(error) => {
                collector
                    .diagnostics
                    .push(ResourceDiagnostic::warning(error.to_string(), &path));
                continue;
            }
        };
        let source = source_for(&path);
        if metadata.is_dir() {
            collector.add(load_skills_from_dir(&path, source));
        } else if metadata.is_file() && has_suffix(&path.to_string_lossy(), ".md") {
            let (skill, diagnostics) = load_skill_from_file(&path, source);
            match skill {
                Some(skill) => collector.add(LoadSkillsResult {
                    skills: vec![skill],
                    diagnostics,
                }),
                None => collector.diagnostics.extend(diagnostics),
            }
        } else {
            collector.diagnostics.push(ResourceDiagnostic::warning(
                "skill path is not a markdown file",
                &path,
            ));
        }
    }
    collector.finish()
}

#[derive(Default)]
struct SkillCollector {
    skills: Vec<Skill>,
    by_name: HashMap<String, usize>,
    real_paths: HashSet<PathBuf>,
    diagnostics: Vec<ResourceDiagnostic>,
    collisions: Vec<ResourceDiagnostic>,
}

impl SkillCollector {
    fn add(&mut self, result: LoadSkillsResult) {
        self.diagnostics.extend(result.diagnostics);
        for skill in result.skills {
            let real = canonicalize_or_self(&skill.file_path);
            if self.real_paths.contains(&real) {
                continue;
            }
            if let Some(&index) = self.by_name.get(&skill.name) {
                self.collisions.push(ResourceDiagnostic::collision(
                    format!("name \"{}\" collision", skill.name),
                    CollisionResource::Skill,
                    &skill.name,
                    &self.skills[index].file_path,
                    &skill.file_path,
                ));
            } else {
                self.by_name.insert(skill.name.clone(), self.skills.len());
                self.real_paths.insert(real);
                self.skills.push(skill);
            }
        }
    }

    fn finish(mut self) -> LoadSkillsResult {
        self.diagnostics.append(&mut self.collisions);
        LoadSkillsResult {
            skills: self.skills,
            diagnostics: self.diagnostics,
        }
    }
}

/// The tool a model uses to read skill files (pi `formatSkillsForPrompt` `fileReadTool`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkillReader {
    /// `read` is declared.
    #[default]
    Read,
    /// Only `bash` is declared.
    Bash,
    /// A reader exists but is only reachable through another tool.
    Indirect,
}

/// pi `formatSkillsForPrompt`: the `<available_skills>` block (leading `\n\n`), excluding
/// `disable-model-invocation` skills; empty when none are visible.
#[must_use]
pub fn format_skills_for_prompt(skills: &[Skill], reader: SkillReader) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "\n\nThe following skills provide specialized instructions for specific tasks.".to_owned(),
        match reader {
            SkillReader::Read => {
                "Use the read tool to load a skill's file when the task matches its description."
            }
            SkillReader::Bash => {
                "Use bash to load a skill's file when the task matches its description."
            }
            SkillReader::Indirect => "Load a skill's file when the task matches its description.",
        }
        .to_owned(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_owned(),
        String::new(),
        "<available_skills>".to_owned(),
    ];
    for skill in visible {
        lines.push("  <skill>".to_owned());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path.to_string_lossy())
        ));
        lines.push("  </skill>".to_owned());
    }
    lines.push("</available_skills>".to_owned());
    lines.join("\n")
}

/// XML-escape `& < > " '` (pi `escapeXml`).
#[must_use]
pub fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Why a `/skill:name` command could not be expanded from its file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SkillExpansionError {
    /// The skill file could not be read.
    #[error("{0}")]
    Read(#[from] std::io::Error),
    /// The skill file's frontmatter is invalid.
    #[error(transparent)]
    Frontmatter(#[from] crate::frontmatter::FrontmatterError),
}

/// A recognised `/skill:name [args]` input (pi `_expandSkillCommand` parsing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillCommand<'a> {
    /// Skill name after `/skill:`.
    pub name: &'a str,
    /// Trimmed arguments after the first space (may be empty).
    pub args: &'a str,
}

/// Parse `/skill:name args`; `None` unless `text` starts with `/skill:`.
#[must_use]
pub fn parse_skill_command(text: &str) -> Option<SkillCommand<'_>> {
    let rest = text.strip_prefix("/skill:")?;
    Some(match text.find(' ') {
        Some(space) => SkillCommand {
            name: &text[7..space],
            args: text[space + 1..].trim(),
        },
        None => SkillCommand {
            name: rest,
            args: "",
        },
    })
}

/// The `<skill …>` block for `skill` with its body and optional user text (pi
/// `_expandSkillCommand` output).
#[must_use]
pub fn render_skill_block(skill: &Skill, body: &str, args: &str) -> String {
    let block = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
        skill.name,
        skill.file_path.display(),
        skill.base_dir.display(),
        body.trim()
    );
    if args.is_empty() {
        block
    } else {
        format!("{block}\n\n{args}")
    }
}

/// pi `_expandSkillCommand`: expand `/skill:name args` with the named skill's body. Returns
/// `Ok(None)` when `text` is not a skill command or the skill is unknown (pass-through).
///
/// # Errors
///
/// [`SkillExpansionError`] when the skill file cannot be read or parsed (pi emits an
/// extension error with event `skill_expansion` and keeps the original text).
pub fn expand_skill_command(
    text: &str,
    skills: &[Skill],
) -> Result<Option<String>, SkillExpansionError> {
    let Some(command) = parse_skill_command(text) else {
        return Ok(None);
    };
    let Some(skill) = skills.iter().find(|skill| skill.name == command.name) else {
        return Ok(None);
    };
    let content = fs::read_to_string(&skill.file_path)?;
    let body = strip_frontmatter(&content)?;
    Ok(Some(render_skill_block(skill, &body, command.args)))
}

/// A parsed skill block (pi `ParsedSkillBlock`, used to render skill invocations).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSkillBlock {
    /// Skill name.
    pub name: String,
    /// Skill file location.
    pub location: String,
    /// Block content (references line + body).
    pub content: String,
    /// Trailing user message, if any.
    pub user_message: Option<String>,
}

/// pi `parseSkillBlock`: recognise text produced by [`render_skill_block`].
#[must_use]
pub fn parse_skill_block(text: &str) -> Option<ParsedSkillBlock> {
    let rest = text.strip_prefix("<skill name=\"")?;
    let name_end = rest.find('"')?;
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    let rest = rest[name_end..].strip_prefix("\" location=\"")?;
    let location_end = rest.find('"')?;
    let location = &rest[..location_end];
    if location.is_empty() {
        return None;
    }
    let rest = rest[location_end..].strip_prefix("\">\n")?;
    // Lazy `([\s\S]*?)\n</skill>` followed by end or `\n\n` + non-empty user text.
    let mut search = 0;
    while let Some(found) = rest[search..].find("\n</skill>") {
        let end = search + found;
        let after = &rest[end + "\n</skill>".len()..];
        let user_message = if after.is_empty() {
            Some(None)
        } else {
            after
                .strip_prefix("\n\n")
                .filter(|message| !message.is_empty())
                .map(|message| {
                    Some(message.trim())
                        .filter(|m| !m.is_empty())
                        .map(str::to_owned)
                })
        };
        if let Some(user_message) = user_message {
            return Some(ParsedSkillBlock {
                name: name.to_owned(),
                location: location.to_owned(),
                content: rest[..end].to_owned(),
                user_message,
            });
        }
        search = end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_messages() {
        assert_eq!(validate_name("ok-name1"), Vec::<String>::new());
        assert_eq!(
            validate_name("-Bad--"),
            vec![
                "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)",
                "name must not start or end with a hyphen",
                "name must not contain consecutive hyphens",
            ]
        );
        assert_eq!(
            validate_name(&"a".repeat(65)),
            vec!["name exceeds 64 characters (65)"]
        );
    }

    #[test]
    fn skill_command_parsing() {
        assert_eq!(
            parse_skill_command("/skill:x  do it "),
            Some(SkillCommand {
                name: "x",
                args: "do it"
            })
        );
        assert_eq!(
            parse_skill_command("/skill:x"),
            Some(SkillCommand {
                name: "x",
                args: ""
            })
        );
        assert_eq!(parse_skill_command("/x"), None);
    }

    #[test]
    fn skill_block_round_trip() {
        let skill = Skill {
            name: "s".into(),
            description: "d".into(),
            file_path: "/a/s/SKILL.md".into(),
            base_dir: "/a/s".into(),
            source_info: SourceInfo::synthetic(Path::new("/a/s/SKILL.md"), "local", None, None),
            disable_model_invocation: false,
        };
        let block = render_skill_block(&skill, "\nBody\n", "go now");
        assert_eq!(
            block,
            "<skill name=\"s\" location=\"/a/s/SKILL.md\">\nReferences are relative to /a/s.\n\nBody\n</skill>\n\ngo now"
        );
        let parsed = parse_skill_block(&block).unwrap();
        assert_eq!(parsed.content, "References are relative to /a/s.\n\nBody");
        assert_eq!(parsed.user_message.as_deref(), Some("go now"));
        assert_eq!(
            parse_skill_block(&render_skill_block(&skill, "B", ""))
                .unwrap()
                .user_message,
            None
        );
    }
}
