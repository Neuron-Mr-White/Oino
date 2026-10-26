//! [`ResourceLoader`] (pi `DefaultResourceLoader`, minus extension loading): skills, prompt
//! templates, theme files, context files and SYSTEM/APPEND prompt sources.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use oino_settings::Settings;
use oino_settings::paths::{
    APPEND_SYSTEM_PROMPT_FILE, SYSTEM_PROMPT_FILE, canonicalize_or_self, is_under_path,
    lexical_normalize, project_dir, resolve_path,
};
use serde::{Deserialize, Serialize};

use crate::context_files::{ContextFile, load_project_context_files};
use crate::diagnostics::{CollisionResource, ResourceDiagnostic};
use crate::discovery::{DiscoveryInput, ResolvedResource, is_local_path, resolve_local_resources};
use crate::skills::{LoadSkillsOptions, Skill, load_skills};
use crate::source::{PathMetadata, SourceInfo, SourceOrigin, SourceScope};
use crate::templates::{LoadPromptTemplatesOptions, PromptTemplate, load_prompt_templates};
use crate::walk::{has_suffix, read_dir_sorted};

/// Construction inputs (pi `DefaultResourceLoaderOptions`, runtime subset).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi CLI flags -ns, -np, --no-themes, -nc"
)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceLoaderOptions {
    /// Working directory (made absolute).
    pub cwd: PathBuf,
    /// Agent dir (made absolute).
    pub agent_dir: PathBuf,
    /// Home dir for `~` and `~/.agents/skills` (`None`: neither is used).
    pub home_dir: Option<PathBuf>,
    /// `-ns`: skip discovered and configured skills (CLI `--skill` paths still load).
    pub no_skills: bool,
    /// `-np`: skip discovered and configured prompt templates.
    pub no_prompt_templates: bool,
    /// Skip discovered and configured themes.
    pub no_themes: bool,
    /// `-nc`: skip context files.
    pub no_context_files: bool,
    /// `--skill` paths (relative to cwd).
    pub skill_paths: Vec<String>,
    /// `--prompt-template` paths.
    pub prompt_template_paths: Vec<String>,
    /// `--theme` paths.
    pub theme_paths: Vec<String>,
    /// `--system-prompt` text or file path.
    pub system_prompt: Option<String>,
    /// `--append-system-prompt` values (text or file paths, in order). `Some` replaces the
    /// discovered `APPEND_SYSTEM.md` (pi parity); `None` discovers it.
    pub append_system_prompt: Option<Vec<String>>,
}

/// Paths contributed after load (pi `ResourceExtensionPaths`; packages and
/// `resources_discover`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceExtensionPaths {
    /// Skill files or dirs.
    pub skill_paths: Vec<ExtraResourcePath>,
    /// Prompt template files or dirs.
    pub prompt_paths: Vec<ExtraResourcePath>,
    /// Theme files or dirs.
    pub theme_paths: Vec<ExtraResourcePath>,
}

/// One contributed path with its provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraResourcePath {
    /// Path (relative to cwd when not absolute).
    pub path: PathBuf,
    /// Provenance metadata.
    pub metadata: PathMetadata,
}

/// A discovered theme file (frontends parse and render themes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeResource {
    /// Theme `name` (`unnamed` when absent).
    pub name: String,
    /// Theme JSON file.
    pub path: PathBuf,
    /// Provenance.
    pub source_info: SourceInfo,
}

/// Loads every markdown/JSON resource for one cwd; call [`Self::reload`] after construction and
/// whenever settings or trust change. Synchronous (run it on a blocking thread).
#[derive(Clone, Debug, Default)]
pub struct ResourceLoader {
    options: ResourceLoaderOptions,
    project_trusted: bool,
    metadata: IndexMap<PathBuf, PathMetadata>,
    extra_skill_infos: IndexMap<PathBuf, SourceInfo>,
    extra_prompt_infos: IndexMap<PathBuf, SourceInfo>,
    extra_theme_infos: IndexMap<PathBuf, SourceInfo>,
    skill_paths: Vec<PathBuf>,
    prompt_paths: Vec<PathBuf>,
    theme_paths: Vec<PathBuf>,
    skills: Vec<Skill>,
    skill_diagnostics: Vec<ResourceDiagnostic>,
    prompts: Vec<PromptTemplate>,
    prompt_diagnostics: Vec<ResourceDiagnostic>,
    themes: Vec<ThemeResource>,
    theme_diagnostics: Vec<ResourceDiagnostic>,
    context_files: Vec<ContextFile>,
    context_diagnostics: Vec<ResourceDiagnostic>,
    system_prompt: Option<String>,
    system_prompt_source: Option<PathBuf>,
    append_system_prompt: Vec<String>,
    append_system_prompt_sources: Vec<PathBuf>,
    prompt_input_diagnostics: Vec<ResourceDiagnostic>,
}

impl ResourceLoader {
    /// A loader with nothing loaded yet.
    #[must_use]
    pub fn new(options: ResourceLoaderOptions) -> Self {
        let absolute = |path: &Path| {
            std::path::absolute(path).map_or_else(|_| path.to_path_buf(), |p| lexical_normalize(&p))
        };
        let options = ResourceLoaderOptions {
            cwd: absolute(&options.cwd),
            agent_dir: absolute(&options.agent_dir),
            ..options
        };
        Self {
            options,
            ..Self::default()
        }
    }

    /// The (absolutised) options.
    #[must_use]
    pub fn options(&self) -> &ResourceLoaderOptions {
        &self.options
    }

    /// Re-resolve everything for `settings` and the trust state (pi `reload`; extension
    /// contributions from [`Self::extend`] are dropped, as in pi).
    pub fn reload(&mut self, settings: &Settings, project_trusted: bool) {
        self.project_trusted = project_trusted;
        let cwd = self.options.cwd.clone();
        let agent_dir = self.options.agent_dir.clone();
        let home = self.options.home_dir.clone();
        let resolved = resolve_local_resources(&DiscoveryInput {
            cwd: &cwd,
            agent_dir: &agent_dir,
            home: home.as_deref(),
            project_trusted,
            settings,
        });
        self.metadata = IndexMap::new();
        self.extra_skill_infos = IndexMap::new();
        self.extra_prompt_infos = IndexMap::new();
        self.extra_theme_infos = IndexMap::new();
        let enabled_skills: Vec<PathBuf> = self
            .enabled(&resolved.skills)
            .into_iter()
            .map(|resource| self.map_skill_path(&resource))
            .collect();
        let enabled_prompts: Vec<PathBuf> = self
            .enabled(&resolved.prompts)
            .into_iter()
            .map(|r| r.path)
            .collect();
        let enabled_themes: Vec<PathBuf> = self
            .enabled(&resolved.themes)
            .into_iter()
            .map(|r| r.path)
            .collect();

        let cli_skills = to_paths(&self.options.skill_paths);
        let cli_prompts = to_paths(&self.options.prompt_template_paths);
        let cli_themes = to_paths(&self.options.theme_paths);

        self.skill_paths = if self.options.no_skills {
            self.merge_paths(&[], &cli_skills)
        } else {
            self.merge_paths(&enabled_skills, &cli_skills)
        };
        self.update_skills();
        for path in &self.options.skill_paths.clone() {
            if is_local_path(path) {
                let resolved = self.resolve_resource_path(Path::new(path));
                if !resolved.exists() && !has_diagnostic_for(&self.skill_diagnostics, &resolved) {
                    self.skill_diagnostics.push(ResourceDiagnostic::error(
                        "Skill path does not exist",
                        resolved,
                    ));
                }
            }
        }

        self.prompt_paths = if self.options.no_prompt_templates {
            self.merge_paths(&[], &cli_prompts)
        } else {
            self.merge_paths(&enabled_prompts, &cli_prompts)
        };
        self.update_prompts();
        for path in &self.options.prompt_template_paths.clone() {
            if is_local_path(path) {
                let resolved = self.resolve_resource_path(Path::new(path));
                if !resolved.exists() && !has_diagnostic_for(&self.prompt_diagnostics, &resolved) {
                    self.prompt_diagnostics.push(ResourceDiagnostic::error(
                        "Prompt template path does not exist",
                        resolved,
                    ));
                }
            }
        }

        self.theme_paths = if self.options.no_themes {
            self.merge_paths(&[], &cli_themes)
        } else {
            self.merge_paths(&enabled_themes, &cli_themes)
        };
        self.update_themes();
        for path in &self.options.theme_paths.clone() {
            let resolved = self.resolve_resource_path(Path::new(path));
            if !resolved.exists() && !has_diagnostic_for(&self.theme_diagnostics, &resolved) {
                self.theme_diagnostics.push(ResourceDiagnostic::error(
                    "Theme path does not exist",
                    resolved,
                ));
            }
        }

        if self.options.no_context_files {
            self.context_files = Vec::new();
            self.context_diagnostics = Vec::new();
        } else {
            let loaded = load_project_context_files(&cwd, &agent_dir);
            self.context_files = loaded.files;
            self.context_diagnostics = loaded.diagnostics;
        }

        self.load_prompt_sources();
    }

    /// Add package / `resources_discover` paths (pi `extendResources`).
    pub fn extend(&mut self, paths: &ResourceExtensionPaths) {
        let normalize =
            |this: &Self, entries: &[ExtraResourcePath]| -> Vec<(PathBuf, SourceInfo)> {
                entries
                    .iter()
                    .map(|entry| {
                        let path = this.resolve_resource_path(&entry.path);
                        let mut metadata = entry.metadata.clone();
                        metadata.base_dir = metadata
                            .base_dir
                            .map(|base| this.resolve_resource_path(&base));
                        let info = SourceInfo::from_metadata(&path, &metadata);
                        (path, info)
                    })
                    .collect()
            };
        let skills = normalize(self, &paths.skill_paths);
        let prompts = normalize(self, &paths.prompt_paths);
        let themes = normalize(self, &paths.theme_paths);
        for (path, info) in &skills {
            self.extra_skill_infos.insert(path.clone(), info.clone());
        }
        for (path, info) in &prompts {
            self.extra_prompt_infos.insert(path.clone(), info.clone());
        }
        for (path, info) in &themes {
            self.extra_theme_infos.insert(path.clone(), info.clone());
        }
        if !skills.is_empty() {
            let added: Vec<PathBuf> = skills.into_iter().map(|(path, _)| path).collect();
            self.skill_paths = self.merge_paths(&self.skill_paths.clone(), &added);
            self.update_skills();
        }
        if !prompts.is_empty() {
            let added: Vec<PathBuf> = prompts.into_iter().map(|(path, _)| path).collect();
            self.prompt_paths = self.merge_paths(&self.prompt_paths.clone(), &added);
            self.update_prompts();
        }
        if !themes.is_empty() {
            let added: Vec<PathBuf> = themes.into_iter().map(|(path, _)| path).collect();
            self.theme_paths = self.merge_paths(&self.theme_paths.clone(), &added);
            self.update_themes();
        }
    }

    /// Loaded skills (first-wins by name).
    #[must_use]
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Skill diagnostics.
    #[must_use]
    pub fn skill_diagnostics(&self) -> &[ResourceDiagnostic] {
        &self.skill_diagnostics
    }

    /// Loaded prompt templates (first-wins by name).
    #[must_use]
    pub fn prompts(&self) -> &[PromptTemplate] {
        &self.prompts
    }

    /// Prompt template diagnostics.
    #[must_use]
    pub fn prompt_diagnostics(&self) -> &[ResourceDiagnostic] {
        &self.prompt_diagnostics
    }

    /// Theme files (first-wins by name).
    #[must_use]
    pub fn themes(&self) -> &[ThemeResource] {
        &self.themes
    }

    /// Theme diagnostics.
    #[must_use]
    pub fn theme_diagnostics(&self) -> &[ResourceDiagnostic] {
        &self.theme_diagnostics
    }

    /// Context files (empty with `-nc`).
    #[must_use]
    pub fn context_files(&self) -> &[ContextFile] {
        &self.context_files
    }

    /// The system prompt override (CLI, trusted `.oino/SYSTEM.md`, `<agent>/SYSTEM.md`).
    #[must_use]
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    /// File the system prompt came from, if any.
    #[must_use]
    pub fn system_prompt_source(&self) -> Option<&Path> {
        self.system_prompt_source.as_deref()
    }

    /// Append texts in order (joined with `\n\n` into the `addendum` section).
    #[must_use]
    pub fn append_system_prompt(&self) -> &[String] {
        &self.append_system_prompt
    }

    /// The `addendum` text: [`Self::append_system_prompt`] joined by `\n\n`.
    #[must_use]
    pub fn addendum(&self) -> String {
        self.append_system_prompt.join("\n\n")
    }

    /// Files the append texts came from.
    #[must_use]
    pub fn append_system_prompt_sources(&self) -> &[PathBuf] {
        &self.append_system_prompt_sources
    }

    /// Every diagnostic: skills, prompts, themes, context files, prompt sources.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<ResourceDiagnostic> {
        [
            &self.skill_diagnostics,
            &self.prompt_diagnostics,
            &self.theme_diagnostics,
            &self.context_diagnostics,
            &self.prompt_input_diagnostics,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }

    fn enabled(&mut self, resources: &[ResolvedResource]) -> Vec<ResolvedResource> {
        for resource in resources {
            self.metadata
                .entry(resource.path.clone())
                .or_insert_with(|| resource.metadata.clone());
        }
        resources.iter().filter(|r| r.enabled).cloned().collect()
    }

    /// pi `mapSkillPath`: an auto/package skill dir with `SKILL.md` maps to that file.
    fn map_skill_path(&mut self, resource: &ResolvedResource) -> PathBuf {
        if resource.metadata.source != "auto" && resource.metadata.origin != SourceOrigin::Package {
            return resource.path.clone();
        }
        if !resource.path.is_dir() {
            return resource.path.clone();
        }
        let skill_file = resource.path.join("SKILL.md");
        if skill_file.exists() {
            self.metadata
                .entry(skill_file.clone())
                .or_insert_with(|| resource.metadata.clone());
            return skill_file;
        }
        resource.path.clone()
    }

    fn resolve_resource_path(&self, path: &Path) -> PathBuf {
        resolve_path(
            &path.to_string_lossy(),
            &self.options.cwd,
            self.options.home_dir.as_deref(),
            true,
        )
    }

    /// pi `mergePaths`: resolve, dedupe by canonical path, keep first.
    fn merge_paths(&self, primary: &[PathBuf], additional: &[PathBuf]) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        primary
            .iter()
            .chain(additional)
            .map(|path| self.resolve_resource_path(path))
            .filter(|path| seen.insert(canonicalize_or_self(path)))
            .collect()
    }

    fn update_skills(&mut self) {
        if self.options.no_skills && self.skill_paths.is_empty() {
            self.skills = Vec::new();
            self.skill_diagnostics = Vec::new();
            return;
        }
        let result = load_skills(&LoadSkillsOptions {
            cwd: &self.options.cwd,
            agent_dir: &self.options.agent_dir,
            home: self.options.home_dir.as_deref(),
            skill_paths: &self.skill_paths,
            include_defaults: false,
        });
        self.skills = result
            .skills
            .into_iter()
            .map(|mut skill| {
                if let Some(info) = self.find_source_info(&skill.file_path, &self.extra_skill_infos)
                {
                    skill.source_info = info;
                }
                skill
            })
            .collect();
        self.skill_diagnostics = result.diagnostics;
    }

    fn update_prompts(&mut self) {
        if self.options.no_prompt_templates && self.prompt_paths.is_empty() {
            self.prompts = Vec::new();
            self.prompt_diagnostics = Vec::new();
            return;
        }
        let loaded = load_prompt_templates(&LoadPromptTemplatesOptions {
            cwd: &self.options.cwd,
            agent_dir: &self.options.agent_dir,
            home: self.options.home_dir.as_deref(),
            prompt_paths: &self.prompt_paths,
            include_defaults: false,
        });
        let mut diagnostics = loaded.diagnostics;
        let mut by_name: IndexMap<String, PromptTemplate> = IndexMap::new();
        for prompt in loaded.templates {
            if let Some(existing) = by_name.get(&prompt.name) {
                diagnostics.push(ResourceDiagnostic::collision(
                    format!("name \"/{}\" collision", prompt.name),
                    CollisionResource::Prompt,
                    &prompt.name,
                    &existing.file_path,
                    &prompt.file_path,
                ));
            } else {
                by_name.insert(prompt.name.clone(), prompt);
            }
        }
        self.prompts = by_name
            .into_values()
            .map(|mut prompt| {
                if let Some(info) =
                    self.find_source_info(&prompt.file_path, &self.extra_prompt_infos)
                {
                    prompt.source_info = info;
                }
                prompt
            })
            .collect();
        self.prompt_diagnostics = diagnostics;
    }

    fn update_themes(&mut self) {
        if self.options.no_themes && self.theme_paths.is_empty() {
            self.themes = Vec::new();
            self.theme_diagnostics = Vec::new();
            return;
        }
        let mut files = Vec::new();
        let mut diagnostics = Vec::new();
        for path in &self.theme_paths {
            let Ok(metadata) = fs::metadata(path) else {
                diagnostics.push(ResourceDiagnostic::warning(
                    "theme path does not exist",
                    path,
                ));
                continue;
            };
            if metadata.is_dir() {
                for entry in read_dir_sorted(path).unwrap_or_default() {
                    if entry.is_file && has_suffix(&entry.name, ".json") {
                        files.push(entry.path);
                    }
                }
            } else if metadata.is_file() && has_suffix(&path.to_string_lossy(), ".json") {
                files.push(path.clone());
            } else {
                diagnostics.push(ResourceDiagnostic::warning(
                    "theme path is not a json file",
                    path,
                ));
            }
        }
        let mut by_name: IndexMap<String, ThemeResource> = IndexMap::new();
        for file in files {
            let name = match theme_name(&file) {
                Ok(name) => name,
                Err(message) => {
                    diagnostics.push(ResourceDiagnostic::warning(message, &file));
                    continue;
                }
            };
            if let Some(existing) = by_name.get(&name) {
                diagnostics.push(ResourceDiagnostic::collision(
                    format!("name \"{name}\" collision"),
                    CollisionResource::Theme,
                    &name,
                    &existing.path,
                    &file,
                ));
                continue;
            }
            let source_info = self
                .find_source_info(&file, &self.extra_theme_infos)
                .unwrap_or_else(|| self.default_source_info(&file));
            by_name.insert(
                name.clone(),
                ThemeResource {
                    name,
                    path: file,
                    source_info,
                },
            );
        }
        self.themes = by_name.into_values().collect();
        self.theme_diagnostics = diagnostics;
    }

    /// pi `findSourceInfoForPath`: contributed paths first, then exact, then ancestor metadata.
    fn find_source_info(
        &self,
        path: &Path,
        extra: &IndexMap<PathBuf, SourceInfo>,
    ) -> Option<SourceInfo> {
        let normalized = lexical_normalize(path);
        if let Some(info) = extra
            .iter()
            .find(|(source, _)| normalized.starts_with(lexical_normalize(source)))
            .map(|(_, info)| info)
        {
            return Some(SourceInfo {
                path: path.to_path_buf(),
                ..info.clone()
            });
        }
        if let Some(metadata) = self
            .metadata
            .get(&normalized)
            .or_else(|| self.metadata.get(path))
        {
            return Some(SourceInfo::from_metadata(path, metadata));
        }
        self.metadata
            .iter()
            .find(|(source, _)| normalized.starts_with(lexical_normalize(source)))
            .map(|(_, metadata)| SourceInfo::from_metadata(path, metadata))
    }

    /// pi `getDefaultSourceInfoForPath`.
    fn default_source_info(&self, path: &Path) -> SourceInfo {
        let normalized = lexical_normalize(path);
        let kinds = ["skills", "prompts", "themes", "extensions"];
        let project = project_dir(&self.options.cwd);
        for (base, scope) in [
            (&self.options.agent_dir, SourceScope::User),
            (&project, SourceScope::Project),
        ] {
            if let Some(root) = kinds
                .iter()
                .map(|kind| base.join(kind))
                .find(|root| is_under_path(&normalized, root))
            {
                return SourceInfo::synthetic(path, "local", Some(scope), Some(root));
            }
        }
        let base = if normalized.is_dir() {
            normalized.clone()
        } else {
            normalized
                .parent()
                .map_or_else(|| normalized.clone(), Path::to_path_buf)
        };
        SourceInfo::synthetic(path, "local", None, Some(base))
    }

    fn load_prompt_sources(&mut self) {
        self.prompt_input_diagnostics = Vec::new();
        let project = project_dir(&self.options.cwd);
        let discover = |name: &str, trusted: bool, agent_dir: &Path| -> Option<String> {
            let project_file = project.join(name);
            if trusted && project_file.exists() {
                return Some(project_file.to_string_lossy().into_owned());
            }
            let global = agent_dir.join(name);
            global
                .exists()
                .then(|| global.to_string_lossy().into_owned())
        };
        let source = self.options.system_prompt.clone().or_else(|| {
            discover(
                SYSTEM_PROMPT_FILE,
                self.project_trusted,
                &self.options.agent_dir,
            )
        });
        self.system_prompt = source
            .as_deref()
            .and_then(|input| self.resolve_prompt_input(input, "system prompt"));
        self.system_prompt_source = source
            .as_deref()
            .and_then(|input| self.existing_path(input));

        let append_sources = self
            .options
            .append_system_prompt
            .clone()
            .unwrap_or_else(|| {
                discover(
                    APPEND_SYSTEM_PROMPT_FILE,
                    self.project_trusted,
                    &self.options.agent_dir,
                )
                .into_iter()
                .collect()
            });
        self.append_system_prompt = append_sources
            .iter()
            .filter_map(|input| self.resolve_prompt_input(input, "append system prompt"))
            .collect();
        self.append_system_prompt_sources = append_sources
            .iter()
            .filter_map(|input| self.existing_path(input))
            .collect();
    }

    fn existing_path(&self, input: &str) -> Option<PathBuf> {
        let path = resolve_path(input, &self.options.cwd, None, false);
        path.exists().then_some(path)
    }

    /// pi `resolvePromptInput`: file content when `input` names an existing file, else the text.
    fn resolve_prompt_input(&mut self, input: &str, description: &str) -> Option<String> {
        if input.is_empty() {
            return None;
        }
        let Some(path) = self.existing_path(input) else {
            return Some(input.to_owned());
        };
        match fs::read_to_string(&path) {
            Ok(content) => Some(
                content
                    .strip_prefix('\u{feff}')
                    .map_or(content.clone(), str::to_owned),
            ),
            Err(error) => {
                self.prompt_input_diagnostics
                    .push(ResourceDiagnostic::warning(
                        format!("Could not read {description} file {input}: {error}"),
                        &path,
                    ));
                Some(input.to_owned())
            }
        }
    }
}

fn to_paths(values: &[String]) -> Vec<PathBuf> {
    values.iter().map(PathBuf::from).collect()
}

fn has_diagnostic_for(diagnostics: &[ResourceDiagnostic], path: &Path) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.path.as_deref() == Some(path))
}

/// The `name` of a theme file (pi's loader validates the full schema in the TUI).
fn theme_name(path: &Path) -> Result<String, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(oino_settings::json::strip_bom(&content))
        .map_err(|e| e.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "theme must be a JSON object".to_owned())?;
    Ok(object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unnamed")
        .to_owned())
}
