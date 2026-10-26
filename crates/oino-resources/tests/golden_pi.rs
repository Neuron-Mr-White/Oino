//! Golden parity with pi 1.4.0 (fixtures from
//! `scripts/pi-oracle/core-runtime/settings-resources/{system-prompt,misc,resources}.mjs`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "golden test asserts by unwrapping"
)]

use std::fs;
use std::path::{Path, PathBuf};

use oino_resources::keybindings::{KeyBinding, is_valid_key_id, migrate_keybindings_config};
use oino_resources::skills::{SkillReader, format_skills_for_prompt, parse_skill_block};
use oino_resources::system_prompt::{
    BuildSystemPromptOptions, build_system_prompt, build_system_prompt_sections,
    build_system_prompt_state,
};
use oino_resources::templates::{parse_command_args, substitute_args};
use oino_resources::{PromptTemplate, ResourceLoader, ResourceLoaderOptions, Skill, SourceInfo};
use oino_settings::SettingsManager;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pi")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

const DOCS_ROOT: &str = "/DOCS";

#[test]
fn system_prompt_matches_pi() {
    for case in fixture("system-prompt-expected.json").as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let options: BuildSystemPromptOptions =
            serde_json::from_value(case["options"].clone()).unwrap();
        let docs = Path::new(DOCS_ROOT);
        if let Some(error) = case.get("error") {
            let got = build_system_prompt_sections(&options, docs).unwrap_err();
            assert_eq!(Value::from(got.to_string()), *error, "{name}");
            continue;
        }
        let sections = build_system_prompt_sections(&options, docs).unwrap();
        assert_eq!(
            serde_json::to_string(&sections).unwrap(),
            serde_json::to_string(&case["sections"]).unwrap(),
            "{name}: sections (order-sensitive)"
        );
        let state = build_system_prompt_state(&options, docs).unwrap();
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            case["state"],
            "{name}: state"
        );
        assert_eq!(
            Value::from(build_system_prompt(&options, docs).unwrap()),
            case["text"],
            "{name}: text"
        );
        // Normalised options round-trip with pi's field names and defaults.
        let normalized = serde_json::to_value(&options).unwrap();
        for (key, want) in case["normalized"].as_object().unwrap() {
            let got = normalized.get(key).cloned().unwrap_or(Value::Null);
            assert_eq!(&got, want, "{name}: normalized {key}");
        }
    }
}

#[test]
fn command_args_and_substitution_match_pi() {
    let misc = fixture("misc-expected.json");
    for case in misc["parseCommandArgs"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            json!(parse_command_args(input)),
            case["output"],
            "parse {input:?}"
        );
    }
    for case in misc["substituteArgs"].as_array().unwrap() {
        let template = case["template"].as_str().unwrap();
        let args: Vec<String> = serde_json::from_value(case["args"].clone()).unwrap();
        assert_eq!(
            json!(substitute_args(template, &args)),
            case["output"],
            "substitute {template:?} {args:?}"
        );
    }
    let templates: Vec<PromptTemplate> = [
        ("review", "Review $1 with ${@:2}"),
        ("all", "All: $ARGUMENTS"),
    ]
    .into_iter()
    .map(|(name, content)| PromptTemplate {
        name: name.into(),
        description: String::new(),
        argument_hint: None,
        content: content.into(),
        source_info: SourceInfo::synthetic(Path::new("/p"), "local", None, None),
        file_path: PathBuf::from(format!("/p/{name}.md")),
    })
    .collect();
    for case in misc["expandPromptTemplate"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let got = oino_resources::templates::expand_prompt_template(input, &templates)
            .unwrap_or_else(|| input.to_owned());
        assert_eq!(json!(got), case["output"], "expand {input:?}");
    }
}

#[test]
fn skill_formatting_and_blocks_match_pi() {
    let misc = fixture("misc-expected.json");
    for case in misc["formatSkills"].as_array().unwrap() {
        let skills: Vec<Skill> = case["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| {
                let mut skill = skill.clone();
                skill["sourceInfo"] = json!({"path": "/", "source": "local", "scope": "temporary", "origin": "top-level"});
                serde_json::from_value(skill).unwrap()
            })
            .collect();
        let reader = match case["reader"].as_str().unwrap() {
            "read" => SkillReader::Read,
            "bash" => SkillReader::Bash,
            _ => SkillReader::Indirect,
        };
        assert_eq!(
            json!(format_skills_for_prompt(&skills, reader)),
            case["output"]
        );
    }
    for case in misc["parseSkillBlock"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let got = parse_skill_block(input).map(|block| {
            let mut value =
                json!({"name": block.name, "location": block.location, "content": block.content});
            if let Some(message) = block.user_message {
                value["userMessage"] = json!(message);
            }
            value
        });
        assert_eq!(
            got.unwrap_or(Value::Null),
            case["output"],
            "block {input:?}"
        );
    }
}

#[test]
fn keybindings_match_pi() {
    let misc = fixture("misc-expected.json");
    for case in misc["keyIdChecks"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        assert_eq!(json!(is_valid_key_id(key)), case["valid"], "key {key:?}");
    }
    for case in misc["keyArrayChecks"].as_array().unwrap() {
        assert_eq!(
            json!(KeyBinding::from_value(&case["value"]).is_some()),
            case["valid"],
            "{}",
            case["value"]
        );
    }
    for case in misc["migrations"].as_array().unwrap() {
        let (config, migrated) = migrate_keybindings_config(case["input"].as_object().unwrap());
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            serde_json::to_string(&case["output"]["config"]).unwrap()
        );
        assert_eq!(json!(migrated), case["output"]["migrated"]);
    }
}

#[test]
fn frontmatter_matches_pi() {
    let misc = fixture("misc-expected.json");
    for case in misc["parseFrontmatter"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let got = oino_resources::frontmatter::parse_frontmatter(input);
        if case.get("error").is_some() {
            assert!(got.is_err(), "{input:?}");
            continue;
        }
        let got = got.unwrap();
        // A scalar YAML document has no fields (pi returns the scalar; no key lookup can hit it).
        let want = match &case["output"]["frontmatter"] {
            object @ Value::Object(_) => object.clone(),
            _ => json!({}),
        };
        assert_eq!(Value::Object(got.fields), want, "{input:?}");
        assert_eq!(json!(got.body), case["output"]["body"], "{input:?}");
    }
}

fn build_tree(root: &Path, tree: &Value) {
    for (rel, content) in tree.as_object().unwrap() {
        let path = root.join(rel.replace("{CONFIG}", ".oino"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content.as_str().unwrap()).unwrap();
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// YAML parser messages differ between the `yaml` npm package and `serde_yaml_ng`.
fn mask_yaml_messages(diagnostics: &mut Value) {
    for diagnostic in diagnostics.as_array_mut().unwrap() {
        if diagnostic["path"]
            .as_str()
            .is_some_and(|p| p.contains("badyaml"))
        {
            diagnostic["message"] = json!("<yaml error>");
        }
    }
}

#[test]
fn resource_loader_matches_pi() {
    let spec = fixture("resources-cases.json");
    let expected = fixture("resources-expected.json");
    for (case, want) in spec["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected.as_array().unwrap())
    {
        let name = case["name"].as_str().unwrap();
        let tmp = tempfile::Builder::new()
            .prefix("oino-resources-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        build_tree(&root, &spec["trees"][case["tree"].as_str().unwrap()]);
        let agent = root.join("agent");
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let cwd = root.join(case["cwd"].as_str().unwrap_or("work/proj"));
        if let Some(settings) = case.get("globalSettings") {
            fs::write(agent.join("settings.json"), settings.to_string()).unwrap();
        }
        if let Some(settings) = case.get("projectSettings") {
            fs::create_dir_all(cwd.join(".oino")).unwrap();
            fs::write(cwd.join(".oino/settings.json"), settings.to_string()).unwrap();
        }
        let trusted = case["trusted"].as_bool().unwrap();
        let settings = SettingsManager::create(&cwd, &agent, trusted);
        let flag = |key: &str| case[key].as_bool().unwrap_or(false);
        let mut loader = ResourceLoader::new(ResourceLoaderOptions {
            cwd: cwd.clone(),
            agent_dir: agent.clone(),
            home_dir: Some(home.clone()),
            no_skills: flag("noSkills"),
            no_prompt_templates: flag("noPromptTemplates"),
            no_themes: true,
            no_context_files: flag("noContextFiles"),
            skill_paths: strings(&case["skillPaths"]),
            prompt_template_paths: strings(&case["promptPaths"]),
            theme_paths: Vec::new(),
            system_prompt: case["systemPrompt"].as_str().map(str::to_owned),
            append_system_prompt: case.get("appendSystemPrompt").map(strings),
        });
        loader.reload(settings.settings(), trusted);

        let mut skill_diagnostics = json!(loader.skill_diagnostics());
        mask_yaml_messages(&mut skill_diagnostics);
        let got = json!({
            "name": name,
            "skills": {"skills": loader.skills(), "diagnostics": skill_diagnostics},
            "prompts": {"prompts": loader.prompts(), "diagnostics": loader.prompt_diagnostics()},
            "agentsFiles": loader.context_files(),
            "systemPrompt": loader.system_prompt(),
            "systemPromptSource": loader.system_prompt_source(),
            "appendSystemPrompt": loader.append_system_prompt(),
            "appendSystemPromptSources": loader.append_system_prompt_sources(),
        });
        let got: Value =
            serde_json::from_str(&got.to_string().replace(root.to_str().unwrap(), "$ROOT"))
                .unwrap();
        let mut want = want.clone();
        mask_yaml_messages(&mut want["skills"]["diagnostics"]);
        for key in [
            "skills",
            "prompts",
            "agentsFiles",
            "systemPrompt",
            "systemPromptSource",
            "appendSystemPrompt",
            "appendSystemPromptSources",
        ] {
            assert_eq!(got[key], want[key], "{name}: {key}");
        }
    }
}
