//! Local resource resolution (pi `DefaultPackageManager.resolve` minus packages): settings
//! resource arrays of both scopes plus auto-discovered directories, with pi's `!glob`, `+path`,
//! `-path` overrides and precedence ordering.
//!
//! Packages and extensions are resolved by `oino-core` (WASM packages); their skill/prompt/theme
//! paths enter through [`crate::ResourceLoader::extend`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;
use indexmap::IndexMap;
use oino_settings::paths::{canonicalize_or_self, normalize_path, project_dir, resolve_path};
use oino_settings::{ResourceKind, Settings, SettingsScope};

use crate::source::{PathMetadata, SourceOrigin, SourceScope};
use crate::walk::{IgnoreMatcher, has_suffix, posix_relative, read_dir_sorted};

/// One resolved resource file (pi `ResolvedResource`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedResource {
    /// File path.
    pub path: PathBuf,
    /// `false` when excluded by a pattern (kept for listings such as `oino config`).
    pub enabled: bool,
    /// Provenance.
    pub metadata: PathMetadata,
}

/// Resolved skills, prompt templates and themes, precedence-sorted and canonically deduplicated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedResources {
    /// Skill files.
    pub skills: Vec<ResolvedResource>,
    /// Prompt template files.
    pub prompts: Vec<ResolvedResource>,
    /// Theme files.
    pub themes: Vec<ResolvedResource>,
}

/// Inputs of [`resolve_local_resources`].
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryInput<'a> {
    /// Absolute working directory.
    pub cwd: &'a Path,
    /// Absolute agent dir.
    pub agent_dir: &'a Path,
    /// Home dir (`~` expansion and `~/.agents/skills`).
    pub home: Option<&'a Path>,
    /// Project trust (gates `.oino/*` and project `.agents/skills`).
    pub project_trusted: bool,
    /// Settings snapshot (project layer empty when untrusted).
    pub settings: &'a Settings,
}

type Accumulator = IndexMap<PathBuf, (PathMetadata, bool)>;

/// Skill discovery flavour (pi `SkillDiscoveryMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillMode {
    /// `<agent>/skills`, `.oino/skills`, settings dirs: root `.md` files are skills.
    Oino,
    /// `.agents/skills`: root `.md` files are not skills, nested ones are.
    Agents,
}

const fn kind_extension(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Themes => ".json",
        _ => ".md",
    }
}

/// pi `resolve()` for skills, prompts and themes.
#[must_use]
pub fn resolve_local_resources(input: &DiscoveryInput<'_>) -> ResolvedResources {
    let global_base = input.agent_dir.to_path_buf();
    let project_base = project_dir(input.cwd);
    let mut skills = Accumulator::new();
    let mut prompts = Accumulator::new();
    let mut themes = Accumulator::new();

    for kind in [
        ResourceKind::Skills,
        ResourceKind::Prompts,
        ResourceKind::Themes,
    ] {
        let target = match kind {
            ResourceKind::Skills => &mut skills,
            ResourceKind::Prompts => &mut prompts,
            _ => &mut themes,
        };
        let project_entries = input
            .settings
            .resource_entries(kind, SettingsScope::Project);
        let global_entries = input.settings.resource_entries(kind, SettingsScope::Global);
        resolve_local_entries(
            input,
            &project_entries,
            kind,
            target,
            &metadata("local", SourceScope::Project, None),
            &project_base,
        );
        resolve_local_entries(
            input,
            &global_entries,
            kind,
            target,
            &metadata("local", SourceScope::User, None),
            &global_base,
        );
    }

    add_auto_discovered(
        input,
        &global_base,
        &project_base,
        &mut skills,
        &mut prompts,
        &mut themes,
    );

    ResolvedResources {
        skills: finish(skills),
        prompts: finish(prompts),
        themes: finish(themes),
    }
}

fn metadata(source: &str, scope: SourceScope, base_dir: Option<&Path>) -> PathMetadata {
    PathMetadata {
        source: source.to_owned(),
        scope,
        origin: SourceOrigin::TopLevel,
        base_dir: base_dir.map(Path::to_path_buf),
    }
}

fn add_resource(target: &mut Accumulator, path: PathBuf, metadata: &PathMetadata, enabled: bool) {
    if path.as_os_str().is_empty() {
        return;
    }
    target
        .entry(path)
        .or_insert_with(|| (metadata.clone(), enabled));
}

/// pi `resourcePrecedenceRank`.
fn precedence(metadata: &PathMetadata) -> u8 {
    if metadata.origin == SourceOrigin::Package {
        return 4;
    }
    let base = if metadata.scope == SourceScope::Project {
        0
    } else {
        2
    };
    base + u8::from(metadata.source != "local")
}

/// pi `toResolvedPaths`: stable precedence sort, then canonical-path deduplication.
fn finish(accumulator: Accumulator) -> Vec<ResolvedResource> {
    let mut resolved: Vec<ResolvedResource> = accumulator
        .into_iter()
        .map(|(path, (metadata, enabled))| ResolvedResource {
            path,
            enabled,
            metadata,
        })
        .collect();
    resolved.sort_by_key(|resource| precedence(&resource.metadata));
    let mut seen = HashSet::new();
    resolved.retain(|resource| seen.insert(canonicalize_or_self(&resource.path)));
    resolved
}

/// pi `resolveLocalEntries`.
fn resolve_local_entries(
    input: &DiscoveryInput<'_>,
    entries: &[String],
    kind: ResourceKind,
    target: &mut Accumulator,
    metadata: &PathMetadata,
    base_dir: &Path,
) {
    if entries.is_empty() {
        return;
    }
    let (patterns, plain): (Vec<&String>, Vec<&String>) =
        entries.iter().partition(|entry| is_pattern(entry));
    let resolved: Vec<PathBuf> = plain
        .iter()
        .map(|entry| resolve_path(entry, base_dir, input.home, true))
        .collect();
    let files = collect_files_from_paths(&resolved, kind);
    let patterns: Vec<&str> = patterns.iter().map(|p| p.as_str()).collect();
    let enabled = apply_patterns(&files, &patterns, base_dir, input.home);
    for file in files {
        let on = enabled.contains(&file);
        add_resource(target, file, metadata, on);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "linear port of pi addAutoDiscoveredResources; order is the contract"
)]
fn add_auto_discovered(
    input: &DiscoveryInput<'_>,
    global_base: &Path,
    project_base: &Path,
    skills: &mut Accumulator,
    prompts: &mut Accumulator,
    themes: &mut Accumulator,
) {
    let settings = input.settings;
    let user_meta = metadata("auto", SourceScope::User, Some(global_base));
    let project_meta = metadata("auto", SourceScope::Project, Some(project_base));
    let overrides =
        |kind: ResourceKind, scope: SettingsScope| settings.resource_entries(kind, scope);
    let home = input.home;
    let add = |target: &mut Accumulator,
               paths: Vec<PathBuf>,
               meta: &PathMetadata,
               overrides: &[String],
               base: &Path| {
        for path in paths {
            let enabled = is_enabled_by_overrides(&path, overrides, base, home);
            add_resource(target, path, meta, enabled);
        }
    };
    let user_agents_skills = home.map(|home| home.join(".agents").join("skills"));
    let project_agents_dirs: Vec<PathBuf> = if input.project_trusted {
        ancestor_agents_skill_dirs(input.cwd)
            .into_iter()
            .filter(|dir| Some(dir) != user_agents_skills.as_ref())
            .collect()
    } else {
        Vec::new()
    };
    let project_skills = overrides(ResourceKind::Skills, SettingsScope::Project);
    let user_skills = overrides(ResourceKind::Skills, SettingsScope::Global);

    if input.project_trusted {
        add(
            skills,
            collect_skill_entries(&project_base.join("skills"), SkillMode::Oino),
            &project_meta,
            &project_skills,
            project_base,
        );
    }
    for dir in &project_agents_dirs {
        let agents_base = dir.parent().unwrap_or(dir);
        let meta = metadata("auto", SourceScope::Project, Some(agents_base));
        add(
            skills,
            collect_skill_entries(dir, SkillMode::Agents),
            &meta,
            &project_skills,
            agents_base,
        );
    }
    if input.project_trusted {
        add(
            prompts,
            collect_direct_files(&project_base.join("prompts"), ".md"),
            &project_meta,
            &overrides(ResourceKind::Prompts, SettingsScope::Project),
            project_base,
        );
        add(
            themes,
            collect_direct_files(&project_base.join("themes"), ".json"),
            &project_meta,
            &overrides(ResourceKind::Themes, SettingsScope::Project),
            project_base,
        );
    }
    add(
        skills,
        collect_skill_entries(&global_base.join("skills"), SkillMode::Oino),
        &user_meta,
        &user_skills,
        global_base,
    );
    if let Some(dir) = &user_agents_skills {
        let agents_base = dir.parent().unwrap_or(dir);
        let meta = metadata("auto", SourceScope::User, Some(agents_base));
        add(
            skills,
            collect_skill_entries(dir, SkillMode::Agents),
            &meta,
            &user_skills,
            agents_base,
        );
    }
    add(
        prompts,
        collect_direct_files(&global_base.join("prompts"), ".md"),
        &user_meta,
        &overrides(ResourceKind::Prompts, SettingsScope::Global),
        global_base,
    );
    add(
        themes,
        collect_direct_files(&global_base.join("themes"), ".json"),
        &user_meta,
        &overrides(ResourceKind::Themes, SettingsScope::Global),
        global_base,
    );
}

/// pi `findGitRepoRoot` (`.git` file or dir).
fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// pi `collectAncestorAgentsSkillDirs`: `.agents/skills` from cwd up to the repo root (or the
/// filesystem root outside a repo).
fn ancestor_agents_skill_dirs(start: &Path) -> Vec<PathBuf> {
    let root = find_git_repo_root(start);
    let mut dirs = Vec::new();
    for dir in start.ancestors() {
        dirs.push(dir.join(".agents").join("skills"));
        if root.as_deref() == Some(dir) {
            break;
        }
    }
    dirs
}

/// pi `collectSkillEntries`.
fn collect_skill_entries(dir: &Path, mode: SkillMode) -> Vec<PathBuf> {
    let mut ignore = IgnoreMatcher::new(dir);
    let mut out = Vec::new();
    skill_entries(dir, mode, &mut ignore, dir, &mut out);
    out
}

fn skill_entries(
    dir: &Path,
    mode: SkillMode,
    ignore: &mut IgnoreMatcher,
    root: &Path,
    out: &mut Vec<PathBuf>,
) {
    if !dir.exists() {
        return;
    }
    ignore.add_rules(dir);
    let Some(entries) = read_dir_sorted(dir) else {
        return;
    };
    if let Some(entry) = entries.iter().find(|entry| entry.name == "SKILL.md")
        && entry.is_file
        && !ignore.ignores(&posix_relative(root, &entry.path), false)
    {
        out.push(entry.path.clone());
        return;
    }
    for entry in &entries {
        if entry.name.starts_with('.') || entry.name == "node_modules" {
            continue;
        }
        let relative = posix_relative(root, &entry.path);
        let at_root = dir == root;
        let include_markdown = entry.is_file
            && has_suffix(&entry.name, ".md")
            && !ignore.ignores(&relative, false)
            && match mode {
                SkillMode::Oino => at_root,
                SkillMode::Agents => !at_root,
            };
        if include_markdown {
            out.push(entry.path.clone());
            continue;
        }
        if !entry.is_dir || ignore.ignores(&relative, true) {
            continue;
        }
        skill_entries(&entry.path, mode, ignore, root, out);
    }
}

/// pi `collectAutoPromptEntries` / `collectAutoThemeEntries`: direct children with `suffix`.
fn collect_direct_files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut ignore = IgnoreMatcher::new(dir);
    ignore.add_rules(dir);
    let Some(entries) = read_dir_sorted(dir) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|entry| !entry.name.starts_with('.') && entry.name != "node_modules")
        .filter(|entry| !ignore.ignores(&posix_relative(dir, &entry.path), false))
        .filter(|entry| entry.is_file && has_suffix(&entry.name, suffix))
        .map(|entry| entry.path)
        .collect()
}

/// pi `collectFiles`: recursive files with `suffix`.
fn collect_files_recursive(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    fn walk(
        dir: &Path,
        suffix: &str,
        ignore: &mut IgnoreMatcher,
        root: &Path,
        out: &mut Vec<PathBuf>,
    ) {
        if !dir.exists() {
            return;
        }
        ignore.add_rules(dir);
        let Some(entries) = read_dir_sorted(dir) else {
            return;
        };
        for entry in entries {
            if entry.name.starts_with('.') || entry.name == "node_modules" {
                continue;
            }
            if ignore.ignores(&posix_relative(root, &entry.path), entry.is_dir) {
                continue;
            }
            if entry.is_dir {
                walk(&entry.path, suffix, ignore, root, out);
            } else if entry.is_file && has_suffix(&entry.name, suffix) {
                out.push(entry.path);
            }
        }
    }
    let mut ignore = IgnoreMatcher::new(dir);
    let mut out = Vec::new();
    walk(dir, suffix, &mut ignore, dir, &mut out);
    out
}

/// pi `collectResourceFiles` (skills use skill discovery, others recursive files).
fn collect_resource_files(dir: &Path, kind: ResourceKind) -> Vec<PathBuf> {
    match kind {
        ResourceKind::Skills => collect_skill_entries(dir, SkillMode::Oino),
        _ => collect_files_recursive(dir, kind_extension(kind)),
    }
}

/// pi `collectFilesFromPaths`.
fn collect_files_from_paths(paths: &[PathBuf], kind: ResourceKind) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        if metadata.is_file() {
            files.push(path.clone());
        } else if metadata.is_dir() {
            files.extend(collect_resource_files(path, kind));
        }
    }
    files
}

/// pi `isPattern`.
fn is_pattern(entry: &str) -> bool {
    entry.starts_with('!')
        || entry.starts_with('+')
        || entry.starts_with('-')
        || entry.contains('*')
        || entry.contains('?')
}

fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `minimatch(text, pattern)` approximation: `*` does not cross `/`, braces and classes work.
fn glob_matches(pattern: &str, text: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .is_ok_and(|glob| glob.compile_matcher().is_match(text))
}

/// Candidate strings a pattern may match for `file` (pi `matchesAnyPattern` targets).
struct MatchTargets {
    relative: String,
    name: String,
    absolute: String,
    skill_parent: Option<(String, String, String)>,
}

impl MatchTargets {
    fn new(file: &Path, base_dir: &Path) -> Self {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let skill_parent = (name == "SKILL.md")
            .then(|| file.parent())
            .flatten()
            .map(|parent| {
                (
                    relative_posix(base_dir, parent),
                    parent
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    posix(parent),
                )
            });
        Self {
            relative: relative_posix(base_dir, file),
            name,
            absolute: posix(file),
            skill_parent,
        }
    }
}

/// Node `path.relative(base, path)` in posix form (with `..` segments when outside `base`).
fn relative_posix(base: &Path, path: &Path) -> String {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = path.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let ups = std::iter::repeat_n("..".to_owned(), base.len() - common);
    let downs = target[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    ups.chain(downs).collect::<Vec<_>>().join("/")
}

fn matches_any_pattern(
    file: &Path,
    patterns: &[&str],
    base_dir: &Path,
    home: Option<&Path>,
) -> bool {
    let targets = MatchTargets::new(file, base_dir);
    patterns.iter().any(|pattern| {
        let pattern = posix(&normalize_path(pattern, home));
        if glob_matches(&pattern, &targets.relative)
            || glob_matches(&pattern, &targets.name)
            || glob_matches(&pattern, &targets.absolute)
        {
            return true;
        }
        targets
            .skill_parent
            .as_ref()
            .is_some_and(|(rel, name, abs)| {
                glob_matches(&pattern, rel)
                    || glob_matches(&pattern, name)
                    || glob_matches(&pattern, abs)
            })
    })
}

fn matches_any_exact_pattern(
    file: &Path,
    patterns: &[&str],
    base_dir: &Path,
    home: Option<&Path>,
) -> bool {
    let targets = MatchTargets::new(file, base_dir);
    patterns.iter().any(|pattern| {
        let stripped = pattern
            .strip_prefix("./")
            .or_else(|| pattern.strip_prefix(".\\"))
            .unwrap_or(pattern);
        let normalized = posix(&normalize_path(stripped, home));
        if normalized == targets.relative || normalized == targets.absolute {
            return true;
        }
        targets
            .skill_parent
            .as_ref()
            .is_some_and(|(rel, _, abs)| &normalized == rel || &normalized == abs)
    })
}

/// pi `isEnabledByOverrides`: `!glob` excludes, `+path` force-includes, `-path` force-excludes.
fn is_enabled_by_overrides(
    file: &Path,
    entries: &[String],
    base_dir: &Path,
    home: Option<&Path>,
) -> bool {
    let strip = |prefix: char| -> Vec<&str> {
        entries
            .iter()
            .filter_map(|entry| entry.strip_prefix(prefix))
            .collect()
    };
    let (excludes, includes, force_excludes) = (strip('!'), strip('+'), strip('-'));
    let mut enabled = true;
    if !excludes.is_empty() && matches_any_pattern(file, &excludes, base_dir, home) {
        enabled = false;
    }
    if !includes.is_empty() && matches_any_exact_pattern(file, &includes, base_dir, home) {
        enabled = true;
    }
    if !force_excludes.is_empty()
        && matches_any_exact_pattern(file, &force_excludes, base_dir, home)
    {
        enabled = false;
    }
    enabled
}

/// pi `applyPatterns`: includes (or all), minus `!` excludes, plus `+` force-includes, minus
/// `-` force-excludes.
fn apply_patterns(
    files: &[PathBuf],
    patterns: &[&str],
    base_dir: &Path,
    home: Option<&Path>,
) -> HashSet<PathBuf> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut force_includes = Vec::new();
    let mut force_excludes = Vec::new();
    for pattern in patterns {
        if let Some(rest) = pattern.strip_prefix('+') {
            force_includes.push(rest);
        } else if let Some(rest) = pattern.strip_prefix('-') {
            force_excludes.push(rest);
        } else if let Some(rest) = pattern.strip_prefix('!') {
            excludes.push(rest);
        } else {
            includes.push(*pattern);
        }
    }
    let mut result: Vec<&PathBuf> = if includes.is_empty() {
        files.iter().collect()
    } else {
        files
            .iter()
            .filter(|file| matches_any_pattern(file, &includes, base_dir, home))
            .collect()
    };
    if !excludes.is_empty() {
        result.retain(|file| !matches_any_pattern(file, &excludes, base_dir, home));
    }
    if !force_includes.is_empty() {
        for file in files {
            if !result.contains(&file)
                && matches_any_exact_pattern(file, &force_includes, base_dir, home)
            {
                result.push(file);
            }
        }
    }
    if !force_excludes.is_empty() {
        result.retain(|file| !matches_any_exact_pattern(file, &force_excludes, base_dir, home));
    }
    result.into_iter().cloned().collect()
}

/// Whether a settings entry names a local path (pi `isLocalPath`).
#[must_use]
pub fn is_local_path(value: &str) -> bool {
    let trimmed = value.trim();
    ![
        "npm:", "git:", "github:", "http:", "https:", "ssh:", "builtin:",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_posix_like_node() {
        assert_eq!(
            relative_posix(Path::new("/a/b"), Path::new("/a/b/c/d")),
            "c/d"
        );
        assert_eq!(relative_posix(Path::new("/a/b"), Path::new("/a/x")), "../x");
    }

    #[test]
    fn patterns_apply_in_pi_order() {
        let files: Vec<PathBuf> = ["/b/s/one/SKILL.md", "/b/s/two/SKILL.md", "/b/s/x.md"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let base = Path::new("/b");
        let on = apply_patterns(&files, &["!s/*", "+s/two"], base, None);
        assert_eq!(on, HashSet::from([PathBuf::from("/b/s/two/SKILL.md")]));
        let on = apply_patterns(&files, &["*.md", "-/b/s/x.md"], base, None);
        assert_eq!(on.len(), 2);
        assert!(!is_enabled_by_overrides(
            &files[0],
            &["!one".into()],
            base,
            None
        ));
        assert!(is_enabled_by_overrides(
            &files[1],
            &["!one".into()],
            base,
            None
        ));
    }
}
