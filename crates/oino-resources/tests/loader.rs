//! Resource loader, trust, keybindings and skill expansion behaviour on real temp trees.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::fs;
use std::path::{Path, PathBuf};

use oino_resources::keybindings::{KeyBinding, KeybindingsFile};
use oino_resources::skills::expand_skill_command;
use oino_resources::source::{PathMetadata, SourceOrigin, SourceScope};
use oino_resources::system_prompt::{BuildSystemPromptOptions, build_system_prompt_sections};
use oino_resources::trust::{
    ExtensionTrustDecision, TrustRequest, TrustResolution, TrustSource, TrustStore,
    has_protected_resources, resolve_project_trust,
};
use oino_resources::{
    ExtraResourcePath, ResourceExtensionPaths, ResourceLoader, ResourceLoaderOptions,
};
use oino_settings::{ProjectTrustDefault, SettingsManager};

struct Tree {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("oino-resources-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir_all(root.join("work/.git")).unwrap();
        fs::write(root.join("work/.git/HEAD"), "ref").unwrap();
        fs::create_dir_all(root.join("work/proj")).unwrap();
        fs::create_dir_all(root.join("agent")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        Self { _tmp: tmp, root }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("work/proj")
    }

    fn agent(&self) -> PathBuf {
        self.root.join("agent")
    }

    fn loader(&self, options: ResourceLoaderOptions, trusted: bool) -> ResourceLoader {
        let settings = SettingsManager::create(&self.cwd(), &self.agent(), trusted);
        let mut loader = ResourceLoader::new(ResourceLoaderOptions {
            cwd: self.cwd(),
            agent_dir: self.agent(),
            home_dir: Some(self.root.join("home")),
            ..options
        });
        loader.reload(settings.settings(), trusted);
        loader
    }
}

#[test]
fn themes_are_discovered_deduplicated_and_gated() {
    let tree = Tree::new();
    tree.write("agent/themes/dark.json", r#"{"name": "dark"}"#);
    tree.write("agent/themes/broken.json", "{");
    tree.write("work/proj/.oino/themes/dark.json", r#"{"name": "dark"}"#);
    tree.write("work/proj/.oino/themes/light.json", r#"{"name": "light"}"#);
    let loader = tree.loader(ResourceLoaderOptions::default(), true);
    let names: Vec<_> = loader.themes().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["dark", "light"]);
    assert_eq!(loader.themes()[0].source_info.scope, SourceScope::Project);
    assert!(
        loader
            .theme_diagnostics()
            .iter()
            .any(|d| d.message == "name \"dark\" collision")
    );
    assert!(
        loader.theme_diagnostics().iter().any(
            |d| d.path.as_deref() == Some(tree.root.join("agent/themes/broken.json").as_path())
        )
    );

    let untrusted = tree.loader(ResourceLoaderOptions::default(), false);
    let names: Vec<_> = untrusted.themes().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["dark"]);
}

#[test]
fn extend_adds_package_skills_with_their_provenance() {
    let tree = Tree::new();
    let pkg = tree.write(
        "pkgs/p/skills/pkg-skill/SKILL.md",
        "---\ndescription: From a package\n---\nbody",
    );
    let mut loader = tree.loader(ResourceLoaderOptions::default(), true);
    assert_eq!(loader.skills().len(), 0);
    loader.extend(&ResourceExtensionPaths {
        skill_paths: vec![ExtraResourcePath {
            path: tree.root.join("pkgs/p/skills"),
            metadata: PathMetadata {
                source: "pkg:p".into(),
                scope: SourceScope::User,
                origin: SourceOrigin::Package,
                base_dir: Some(tree.root.join("pkgs/p")),
            },
        }],
        ..ResourceExtensionPaths::default()
    });
    assert_eq!(loader.skills().len(), 1);
    let skill = &loader.skills()[0];
    assert_eq!(skill.file_path, pkg);
    assert_eq!(skill.source_info.source, "pkg:p");
    assert_eq!(skill.source_info.origin, SourceOrigin::Package);
}

#[test]
fn skill_command_expansion_reads_the_file() {
    let tree = Tree::new();
    let file = tree.write(
        "agent/skills/demo/SKILL.md",
        "---\ndescription: Demo\n---\n\nDo the thing.\n",
    );
    let loader = tree.loader(ResourceLoaderOptions::default(), true);
    let expanded = expand_skill_command("/skill:demo  now please ", loader.skills())
        .unwrap()
        .unwrap();
    let dir = file.parent().unwrap();
    assert_eq!(
        expanded,
        format!(
            "<skill name=\"demo\" location=\"{}\">\nReferences are relative to {}.\n\nDo the thing.\n</skill>\n\nnow please",
            file.display(),
            dir.display()
        )
    );
    assert_eq!(
        expand_skill_command("/skill:unknown", loader.skills()).unwrap(),
        None
    );
    assert_eq!(
        expand_skill_command("hello", loader.skills()).unwrap(),
        None
    );
}

#[test]
fn loader_output_feeds_the_system_prompt() {
    let tree = Tree::new();
    tree.write("work/proj/AGENTS.md", "\u{feff}Be nice.");
    tree.write("agent/APPEND_SYSTEM.md", "Appended.");
    tree.write("agent/skills/s/SKILL.md", "---\ndescription: S\n---\n");
    let loader = tree.loader(ResourceLoaderOptions::default(), true);
    assert_eq!(loader.context_files()[0].content, "Be nice.");
    let options = BuildSystemPromptOptions {
        cwd: tree.cwd().to_string_lossy().into_owned(),
        custom_prompt: loader.system_prompt().map(str::to_owned),
        append_system_prompt: loader.addendum(),
        context_files: loader.context_files().to_vec(),
        skills: loader.skills().to_vec(),
        ..BuildSystemPromptOptions::default()
    };
    let sections = build_system_prompt_sections(&options, Path::new("/docs")).unwrap();
    let names: Vec<_> = sections.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "preamble",
            "tools",
            "rules",
            "docs",
            "addendum",
            "project_context",
            "skills",
            "cwd"
        ]
    );
    assert_eq!(sections["addendum"], "<addendum>\nAppended.\n</addendum>");
}

#[test]
fn cli_append_replaces_discovered_append_file() {
    let tree = Tree::new();
    tree.write("agent/APPEND_SYSTEM.md", "discovered");
    let loader = tree.loader(
        ResourceLoaderOptions {
            append_system_prompt: Some(vec!["one".into(), "two".into()]),
            ..ResourceLoaderOptions::default()
        },
        true,
    );
    assert_eq!(loader.addendum(), "one\n\ntwo");
    assert_eq!(loader.append_system_prompt_sources().len(), 0);
}

#[test]
fn trust_store_saves_sorted_canonical_keys_and_finds_nearest() {
    let tree = Tree::new();
    let store = TrustStore::new(&tree.agent());
    let work = tree.root.join("work");
    store.set(&work, Some(true)).unwrap();
    store.set(&tree.cwd(), Some(false)).unwrap();
    let text = fs::read_to_string(store.path()).unwrap();
    assert!(text.ends_with("}\n"));
    let keys: Vec<String> =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&text)
            .unwrap()
            .keys()
            .cloned()
            .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert_eq!(store.get(&tree.cwd()).unwrap(), Some(false));
    assert_eq!(store.get(&tree.cwd().join("sub")).unwrap(), Some(false));
    store.set(&tree.cwd(), None).unwrap();
    let entry = store.get_entry(&tree.cwd()).unwrap().unwrap();
    assert_eq!(entry.path, work);
    assert!(entry.trusted);
    fs::write(store.path(), r#"{"/x": 1}"#).unwrap();
    assert_eq!(
        store.get(&tree.cwd()).unwrap_err().to_string(),
        format!(
            "Invalid trust store {}: value for \"/x\" must be true, false, or null",
            store.path().display()
        )
    );
}

#[test]
fn trust_resolution_follows_pi_order() {
    let tree = Tree::new();
    let store = TrustStore::new(&tree.agent());
    let home = oino_settings::paths::home_dir();
    let cwd = tree.cwd();
    let request = |cli, extension, default, has_ui| TrustRequest {
        cwd: &cwd,
        home: home.as_deref(),
        cli_override: cli,
        extension_decision: extension,
        default_trust: default,
        has_ui,
    };
    let decided = |trusted, source| TrustResolution::Decided { trusted, source };
    assert_eq!(
        resolve_project_trust(
            &request(Some(false), None, ProjectTrustDefault::Always, true),
            &store
        )
        .unwrap(),
        decided(false, TrustSource::Cli)
    );
    if !has_protected_resources(&tree.cwd(), home.as_deref()) {
        assert_eq!(
            resolve_project_trust(
                &request(None, None, ProjectTrustDefault::Never, true),
                &store
            )
            .unwrap(),
            decided(true, TrustSource::NoProtectedResources)
        );
    }
    tree.write("work/proj/.oino/SYSTEM.md", "x");
    assert!(has_protected_resources(&tree.cwd(), home.as_deref()));
    assert_eq!(
        resolve_project_trust(
            &request(None, None, ProjectTrustDefault::Ask, false),
            &store
        )
        .unwrap(),
        decided(false, TrustSource::NonInteractive)
    );
    let TrustResolution::Ask { options } =
        resolve_project_trust(&request(None, None, ProjectTrustDefault::Ask, true), &store)
            .unwrap()
    else {
        panic!("expected a prompt");
    };
    let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels[0], "Trust");
    assert!(labels[1].starts_with("Trust parent folder ("));
    assert_eq!(
        &labels[2..],
        [
            "Trust (this session only)",
            "Do not trust",
            "Do not trust (this session only)"
        ]
    );
    let remember = ExtensionTrustDecision {
        trusted: true,
        remember: true,
    };
    assert_eq!(
        resolve_project_trust(
            &request(None, Some(remember), ProjectTrustDefault::Never, true),
            &store
        )
        .unwrap(),
        decided(true, TrustSource::Extension)
    );
    assert_eq!(
        resolve_project_trust(
            &request(None, None, ProjectTrustDefault::Never, true),
            &store
        )
        .unwrap(),
        decided(true, TrustSource::Saved)
    );
}

#[test]
fn protected_resources_include_project_agents_skills() {
    let tree = Tree::new();
    tree.write(
        "work/.agents/skills/x/SKILL.md",
        "---\ndescription: d\n---\n",
    );
    assert!(has_protected_resources(
        &tree.cwd(),
        Some(&tree.root.join("home"))
    ));
}

#[test]
fn keybindings_file_migrates_and_drops_invalid_values() {
    let tree = Tree::new();
    tree.write(
        "agent/keybindings.json",
        r#"{"$schema": "x", "cursorUp": ["up", "ctrl+p"], "app.exit": "Bad Key", "app.clear": "ctrl+l"}"#,
    );
    let file = KeybindingsFile::load(&tree.agent());
    assert!(file.migrated);
    assert_eq!(
        file.bindings.keys().map(String::as_str).collect::<Vec<_>>(),
        ["tui.editor.cursorUp", "app.clear"]
    );
    assert_eq!(
        file.bindings["tui.editor.cursorUp"],
        KeyBinding::Many(vec!["up".into(), "ctrl+p".into()])
    );
    assert_eq!(file.diagnostics.len(), 1);
    let missing = KeybindingsFile::load(&tree.root.join("nowhere"));
    assert_eq!(missing.bindings.len(), 0);
    assert_eq!(missing.diagnostics.len(), 0);
}
