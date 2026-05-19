#![doc = r#"Oino-owned resource discovery and loading.

This crate intentionally discovers only explicit Oino paths. Compatibility with Pi,
Claude, Codex, or generic Agent conventions should happen through future importers,
not through silent startup discovery.
"#]
#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

const SYSTEM_DEFAULT: &str = r#"# Oino System Prompt

You are an expert coding assistant operating inside Oino, a terminal coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.

## Available tools

- read: Read file contents
- bash: Execute bash commands (ls, rg, find, etc.)
- edit: Make precise file edits with exact text replacement
- write: Create or overwrite files

## Guidelines

- Use bash for file operations like ls, rg, find.
- Use read to examine files instead of cat or sed.
- Use edit for precise changes; oldText must match exactly and uniquely.
- Use write only for new files or complete rewrites.
- Be concise in your responses.
- Show file paths clearly when working with files.

Oino appends project instructions, available skills, and the current working directory after this file.
"#;
const AGENT_DEFAULT: &str = "# Oino Project Instructions\n\nThis file controls Oino's behavior for this project.\nAdd build commands, coding conventions, architecture notes, and constraints here.\n";
const SETTINGS_DEFAULT: &str = "{}\n";

#[derive(Debug, Error)]
pub enum ResourceError {
    #[error("home directory unavailable for Oino resources")]
    HomeUnavailable,
    #[error("resource io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

pub type ResourceResult<T> = Result<T, ResourceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceScope {
    Global,
    Project,
}

impl ResourceScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePaths {
    pub home_dir: PathBuf,
    pub global_dir: PathBuf,
    pub global_system_prompt: PathBuf,
    pub global_settings: PathBuf,
    pub global_skills_dir: PathBuf,
    pub project_root: PathBuf,
    pub project_dir: PathBuf,
    pub project_settings: PathBuf,
    pub project_agent: PathBuf,
    pub project_prompts_dir: PathBuf,
    pub project_skills_dir: PathBuf,
    pub project_exports_dir: PathBuf,
}

impl ResourcePaths {
    pub fn for_cwd(cwd: impl AsRef<Path>) -> ResourceResult<Self> {
        let Some(home_dir) = dirs::home_dir() else {
            return Err(ResourceError::HomeUnavailable);
        };
        Self::from_home_and_cwd(home_dir, cwd)
    }

    pub fn from_home_and_cwd(
        home_dir: impl Into<PathBuf>,
        cwd: impl AsRef<Path>,
    ) -> ResourceResult<Self> {
        let home_dir = home_dir.into();
        let project_root = find_project_root(cwd.as_ref());
        let global_dir = home_dir.join(".oino");
        let project_dir = project_root.join(".oino");
        Ok(Self {
            global_system_prompt: global_dir.join("SYSTEM.md"),
            global_settings: global_dir.join("settings.json"),
            global_skills_dir: global_dir.join("skills"),
            project_settings: project_dir.join("settings.json"),
            project_agent: project_dir.join("AGENT.md"),
            project_prompts_dir: project_dir.join("prompts"),
            project_skills_dir: project_dir.join("skills"),
            project_exports_dir: project_dir.join("exports"),
            home_dir,
            global_dir,
            project_root,
            project_dir,
        })
    }

    pub fn ensure_skeleton(&self) -> ResourceResult<()> {
        create_dir(&self.global_dir)?;
        create_dir(&self.global_skills_dir)?;
        create_dir(&self.project_dir)?;
        create_dir(&self.project_prompts_dir)?;
        create_dir(&self.project_skills_dir)?;
        create_dir(&self.project_exports_dir)?;
        write_if_missing(&self.global_system_prompt, SYSTEM_DEFAULT)?;
        write_if_missing(&self.global_settings, SETTINGS_DEFAULT)?;
        write_if_missing_inheriting(
            &self.project_settings,
            &self.global_settings,
            SETTINGS_DEFAULT,
        )?;
        write_if_missing(&self.project_agent, AGENT_DEFAULT)?;
        Ok(())
    }

    pub fn load_catalog(&self) -> ResourceCatalog {
        let mut diagnostics = Vec::new();
        let system_prompt = read_optional(&self.global_system_prompt, &mut diagnostics);
        let project_instructions = read_optional(&self.project_agent, &mut diagnostics);
        let prompts = discover_prompts(&self.project_prompts_dir, &mut diagnostics);
        let skills = discover_skills(self, &mut diagnostics);
        ResourceCatalog {
            paths: self.clone(),
            system_prompt,
            project_instructions,
            prompts,
            skills,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCatalog {
    pub paths: ResourcePaths,
    pub system_prompt: Option<String>,
    pub project_instructions: Option<String>,
    pub prompts: Vec<PromptTemplate>,
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<ResourceDiagnostic>,
}

impl ResourceCatalog {
    #[must_use]
    pub fn system_prompt_sections(&self) -> Vec<ResourceSection> {
        let mut sections = Vec::new();
        if let Some(content) = non_empty_content(self.system_prompt.as_deref()) {
            sections.push(ResourceSection {
                title: "Global SYSTEM.md".into(),
                path: self.paths.global_system_prompt.clone(),
                content: content.to_string(),
            });
        }
        if let Some(content) = non_empty_content(self.project_instructions.as_deref()) {
            sections.push(ResourceSection {
                title: "Project AGENT.md".into(),
                path: self.paths.project_agent.clone(),
                content: content.to_string(),
            });
        }
        sections
    }

    #[must_use]
    pub fn prompt_by_name(&self, name: &str) -> Option<&PromptTemplate> {
        self.prompts.iter().find(|prompt| prompt.name == name)
    }

    #[must_use]
    pub fn skill_by_name(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    #[must_use]
    pub fn diagnostics_summary(&self) -> Option<String> {
        if self.diagnostics.is_empty() {
            return None;
        }
        Some(
            self.diagnostics
                .iter()
                .map(ResourceDiagnostic::message)
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSection {
    pub title: String,
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub path: PathBuf,
    pub scope: ResourceScope,
    pub content: String,
}

impl PromptTemplate {
    #[must_use]
    pub fn command(&self) -> String {
        format!("/prompt:{}", self.name)
    }

    #[must_use]
    pub fn expand(&self, input: &str) -> String {
        expand_template(&self.content, input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub base_dir: PathBuf,
    pub scope: ResourceScope,
    pub content: String,
    pub disable_model_invocation: bool,
}

impl Skill {
    #[must_use]
    pub fn command(&self) -> String {
        format!("/skill:{}", self.name)
    }

    #[must_use]
    pub fn invocation_prompt(&self, args: &str) -> String {
        if args.trim().is_empty() {
            format!(
                "Use the `{}` skill.\n\nSkill file: {}\n\n{}",
                self.name,
                self.path.display(),
                self.content
            )
        } else {
            format!(
                "Use the `{}` skill with this user input:\n\n{}\n\nSkill file: {}\n\n{}",
                self.name,
                args.trim(),
                self.path.display(),
                self.content
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceDiagnostic {
    Io {
        path: PathBuf,
        message: String,
    },
    InvalidPrompt {
        path: PathBuf,
        message: String,
    },
    InvalidSkill {
        path: PathBuf,
        message: String,
    },
    DuplicateSkill {
        name: String,
        kept: PathBuf,
        skipped: PathBuf,
    },
}

impl ResourceDiagnostic {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Io { path, message } => format!("{}: {message}", path.display()),
            Self::InvalidPrompt { path, message } => {
                format!("prompt {}: {message}", path.display())
            }
            Self::InvalidSkill { path, message } => {
                format!("skill {}: {message}", path.display())
            }
            Self::DuplicateSkill {
                name,
                kept,
                skipped,
            } => format!(
                "duplicate skill `{name}`: kept {}, skipped {}",
                kept.display(),
                skipped.display()
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frontmatter<'a> {
    fields: BTreeMap<String, String>,
    body: &'a str,
    has_frontmatter: bool,
}

impl<'a> Frontmatter<'a> {
    fn content(&self) -> String {
        if self.has_frontmatter {
            normalized_frontmatter_body(self.body)
        } else {
            self.body.to_string()
        }
    }
}

fn find_project_root(cwd: &Path) -> PathBuf {
    let start = if cwd.as_os_str().is_empty() {
        Path::new(".")
    } else {
        cwd
    };
    let absolute = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    for ancestor in absolute.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }
    absolute
}

fn create_dir(path: &Path) -> ResourceResult<()> {
    fs::create_dir_all(path).map_err(|source| ResourceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_if_missing(path: &Path, content: &str) -> ResourceResult<()> {
    if path.exists() {
        return Ok(());
    }
    write_file(path, content)
}

fn write_if_missing_inheriting(
    path: &Path,
    source_path: &Path,
    fallback: &str,
) -> ResourceResult<()> {
    if path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(source_path).unwrap_or_else(|_| fallback.to_string());
    write_file(path, &content)
}

fn write_file(path: &Path, content: &str) -> ResourceResult<()> {
    if let Some(parent) = path.parent() {
        create_dir(parent)?;
    }
    fs::write(path, content).map_err(|source| ResourceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_optional(path: &Path, diagnostics: &mut Vec<ResourceDiagnostic>) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            diagnostics.push(ResourceDiagnostic::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
            None
        }
    }
}

fn discover_prompts(
    prompts_dir: &Path,
    diagnostics: &mut Vec<ResourceDiagnostic>,
) -> Vec<PromptTemplate> {
    let mut prompts = Vec::new();
    let entries = match fs::read_dir(prompts_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return prompts,
        Err(error) => {
            diagnostics.push(ResourceDiagnostic::Io {
                path: prompts_dir.to_path_buf(),
                message: error.to_string(),
            });
            return prompts;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some(OsStr::new("md")) || !path.is_file() {
            continue;
        }
        match load_prompt(&path) {
            Ok(prompt) => prompts.push(prompt),
            Err(message) => diagnostics.push(ResourceDiagnostic::InvalidPrompt { path, message }),
        }
    }
    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    prompts
}

fn load_prompt(path: &Path) -> Result<PromptTemplate, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let parsed = parse_frontmatter(&raw);
    let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
        return Err("missing UTF-8 file name".into());
    };
    let name = stem.to_string();
    if !is_resource_name(&name) {
        return Err(
            "file name must use lowercase letters, numbers, hyphens, or underscores".into(),
        );
    }
    let description = parsed
        .fields
        .get("description")
        .cloned()
        .or_else(|| first_non_empty_line(parsed.body))
        .unwrap_or_else(|| "Prompt template".into());
    let argument_hint = parsed.fields.get("argument-hint").cloned();
    Ok(PromptTemplate {
        name,
        description,
        argument_hint,
        path: path.to_path_buf(),
        scope: ResourceScope::Project,
        content: parsed.content(),
    })
}

fn discover_skills(paths: &ResourcePaths, diagnostics: &mut Vec<ResourceDiagnostic>) -> Vec<Skill> {
    let mut discovered = Vec::new();
    let mut seen = BTreeMap::<String, PathBuf>::new();
    collect_skills(
        &paths.project_skills_dir,
        ResourceScope::Project,
        &mut discovered,
        &mut seen,
        diagnostics,
    );
    collect_skills(
        &paths.global_skills_dir,
        ResourceScope::Global,
        &mut discovered,
        &mut seen,
        diagnostics,
    );
    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    discovered
}

fn collect_skills(
    root: &Path,
    scope: ResourceScope,
    skills: &mut Vec<Skill>,
    seen: &mut BTreeMap<String, PathBuf>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            diagnostics.push(ResourceDiagnostic::Io {
                path: root.to_path_buf(),
                message: error.to_string(),
            });
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skill_path = path.join("SKILL.md");
            if skill_path.is_file() {
                match load_skill(&skill_path, scope) {
                    Ok(skill) => {
                        if let Some(kept) = seen.get(&skill.name) {
                            diagnostics.push(ResourceDiagnostic::DuplicateSkill {
                                name: skill.name,
                                kept: kept.clone(),
                                skipped: skill_path,
                            });
                        } else {
                            seen.insert(skill.name.clone(), skill.path.clone());
                            skills.push(skill);
                        }
                    }
                    Err(message) => diagnostics.push(ResourceDiagnostic::InvalidSkill {
                        path: skill_path,
                        message,
                    }),
                }
            } else {
                collect_skills(&path, scope, skills, seen, diagnostics);
            }
        }
    }
}

fn load_skill(path: &Path, scope: ResourceScope) -> Result<Skill, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let parsed = parse_frontmatter(&raw);
    let name = parsed
        .fields
        .get("name")
        .cloned()
        .ok_or_else(|| "missing required `name` frontmatter".to_string())?;
    if !is_skill_name(&name) {
        return Err("skill name must use lowercase letters, numbers, and hyphens".into());
    }
    let description = parsed
        .fields
        .get("description")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "missing required `description` frontmatter".to_string())?;
    let expected_dir = path
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    if expected_dir != Some(name.as_str()) {
        return Err("skill name must match parent directory".into());
    }
    let disable_model_invocation = parsed
        .fields
        .get("disable-model-invocation")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let base_dir = path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "skill file has no parent directory".to_string())?;
    Ok(Skill {
        name,
        description,
        path: path.to_path_buf(),
        base_dir,
        scope,
        content: raw,
        disable_model_invocation,
    })
}

fn parse_frontmatter(raw: &str) -> Frontmatter<'_> {
    let mut segments = raw.split_inclusive('\n');
    let Some(first) = segments.next() else {
        return Frontmatter {
            fields: BTreeMap::new(),
            body: raw,
            has_frontmatter: false,
        };
    };
    if logical_line(first) != "---" {
        return Frontmatter {
            fields: BTreeMap::new(),
            body: raw,
            has_frontmatter: false,
        };
    }

    let mut fields = BTreeMap::new();
    let mut offset = first.len();
    for segment in segments {
        let line = logical_line(segment);
        offset += segment.len();
        if line == "---" {
            return Frontmatter {
                fields,
                body: &raw[offset..],
                has_frontmatter: true,
            };
        }
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(
                key.trim().to_ascii_lowercase(),
                trim_frontmatter_value(value).to_string(),
            );
        }
    }

    Frontmatter {
        fields: BTreeMap::new(),
        body: raw,
        has_frontmatter: false,
    }
}

fn logical_line(segment: &str) -> &str {
    segment.trim_end_matches('\n').trim_end_matches('\r')
}

fn normalized_frontmatter_body(body: &str) -> String {
    let mut lines = body.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(body.len());
    out.push_str(first);
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    out
}

fn trim_frontmatter_value(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'')
}

fn first_non_empty_line(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn non_empty_content(content: Option<&str>) -> Option<&str> {
    content.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn is_resource_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn is_skill_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn expand_template(template: &str, input: &str) -> String {
    let args = shell_words(input);
    let mut expanded = template
        .replace("$ARGUMENTS", input.trim())
        .replace("$@", input.trim());
    for index in (1..=9).rev() {
        let needle = format!("${index}");
        let value = args.get(index - 1).map_or("", String::as_str);
        expanded = expanded.replace(&needle, value);
    }
    expanded
}

fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in input.chars() {
        match (quote, ch) {
            (Some(active), c) if c == active => quote = None,
            (None, '\"' | '\'') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn frontmatter_parser_avoids_body_changes() {
        let parsed = parse_frontmatter("---\ndescription: Test\n---\nBody\n\n");
        assert!(parsed.has_frontmatter);
        assert_eq!(
            parsed.fields.get("description").map(String::as_str),
            Some("Test")
        );
        assert_eq!(parsed.content(), "Body\n");

        let raw = "No frontmatter\nkeeps trailing newline\n";
        let parsed = parse_frontmatter(raw);
        assert!(!parsed.has_frontmatter);
        assert_eq!(parsed.content(), raw);
    }

    #[test]
    fn frontmatter_parser_accepts_crlf_delimiters() {
        let parsed = parse_frontmatter("---\r\ndescription: Test\r\n---\r\nBody\r\n");
        assert!(parsed.has_frontmatter);
        assert_eq!(
            parsed.fields.get("description").map(String::as_str),
            Some("Test")
        );
        assert_eq!(parsed.content(), "Body");
    }

    #[test]
    fn skeleton_creation_is_visible_and_does_not_overwrite(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        fs::create_dir_all(&project)?;
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)?;
        paths.ensure_skeleton()?;

        assert!(paths.global_system_prompt.is_file());
        assert!(paths.global_settings.is_file());
        assert!(paths.global_skills_dir.is_dir());
        assert!(paths.project_settings.is_file());
        assert!(paths.project_agent.is_file());
        assert!(paths.project_prompts_dir.is_dir());
        assert!(paths.project_skills_dir.is_dir());
        assert!(paths.project_exports_dir.is_dir());

        fs::write(&paths.global_system_prompt, "custom")?;
        paths.ensure_skeleton()?;
        assert_eq!(fs::read_to_string(&paths.global_system_prompt)?, "custom");
        Ok(())
    }

    #[test]
    fn new_project_settings_inherit_global_settings() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let home = temp.path().join("home");
        let first_project = temp.path().join("first");
        fs::create_dir_all(&first_project)?;
        let first_paths = ResourcePaths::from_home_and_cwd(&home, &first_project)?;
        first_paths.ensure_skeleton()?;
        fs::write(
            &first_paths.global_settings,
            "{\n  \"tools\": {\n    \"bash\": false\n  }\n}\n",
        )?;

        let second_project = temp.path().join("second");
        fs::create_dir_all(&second_project)?;
        let second_paths = ResourcePaths::from_home_and_cwd(&home, &second_project)?;
        second_paths.ensure_skeleton()?;

        assert_eq!(
            fs::read_to_string(&second_paths.project_settings)?,
            fs::read_to_string(&first_paths.global_settings)?
        );
        Ok(())
    }

    #[test]
    fn project_root_uses_git_root() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let nested = project.join("a/b");
        fs::create_dir_all(project.join(".git"))?;
        fs::create_dir_all(&nested)?;
        let paths = ResourcePaths::from_home_and_cwd(&home, &nested)?;
        assert_eq!(paths.project_root, project.canonicalize()?);
        Ok(())
    }

    #[test]
    fn discovers_project_prompts_and_expands_arguments() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let paths = ResourcePaths::from_home_and_cwd(temp.path().join("home"), temp.path())?;
        paths.ensure_skeleton()?;
        fs::write(
            paths.project_prompts_dir.join("review.md"),
            "---\ndescription: Review changes\nargument-hint: \"[focus]\"\n---\nReview $1 and $ARGUMENTS",
        )?;

        let catalog = paths.load_catalog();
        assert_eq!(catalog.prompts.len(), 1);
        let prompt = catalog
            .prompt_by_name("review")
            .ok_or("missing review prompt")?;
        assert_eq!(prompt.description, "Review changes");
        assert_eq!(prompt.argument_hint.as_deref(), Some("[focus]"));
        assert_eq!(
            prompt.expand("bugs security"),
            "Review bugs and bugs security"
        );
        Ok(())
    }

    #[test]
    fn discovers_project_skill_before_global_duplicate() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let paths = ResourcePaths::from_home_and_cwd(temp.path().join("home"), temp.path())?;
        paths.ensure_skeleton()?;
        let project_skill = paths.project_skills_dir.join("debug");
        let global_skill = paths.global_skills_dir.join("debug");
        fs::create_dir_all(&project_skill)?;
        fs::create_dir_all(&global_skill)?;
        fs::write(
            project_skill.join("SKILL.md"),
            "---\nname: debug\ndescription: Project debug\n---\n# Debug\n",
        )?;
        fs::write(
            global_skill.join("SKILL.md"),
            "---\nname: debug\ndescription: Global debug\n---\n# Debug\n",
        )?;

        let catalog = paths.load_catalog();
        assert_eq!(catalog.skills.len(), 1);
        assert_eq!(catalog.skills[0].description, "Project debug");
        assert!(matches!(
            catalog.diagnostics.first(),
            Some(ResourceDiagnostic::DuplicateSkill { name, .. }) if name == "debug"
        ));
        Ok(())
    }
}
