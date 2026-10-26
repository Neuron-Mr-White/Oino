//! `SettingsManager` behaviour beyond the pi golden cases: locking, concurrency, trust
//! switching, reload, Oino keys.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::fs;
use std::path::{Path, PathBuf};

use oino_settings::{
    ProjectTrustDefault, ResourceKind, SettingsErrorKind, SettingsManager, SettingsScope, UiDensity,
};
use serde_json::{Value, json};

struct Dirs {
    _root: tempfile::TempDir,
    agent: PathBuf,
    cwd: PathBuf,
}

fn dirs() -> Dirs {
    let root = tempfile::Builder::new()
        .prefix("oino-settings-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let agent = root.path().join("agent");
    let cwd = root.path().join("cwd");
    fs::create_dir_all(&cwd).unwrap();
    Dirs {
        _root: root,
        agent,
        cwd,
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn writes_create_dirs_lock_file_and_keep_other_writers_keys() {
    let d = dirs();
    let mut a = SettingsManager::create(&d.cwd, &d.agent, true);
    let mut b = SettingsManager::create(&d.cwd, &d.agent, true);
    a.set_default_model("one").unwrap();
    b.set_default_provider("two").unwrap();
    let file = d.agent.join("settings.json");
    assert_eq!(
        read_json(&file),
        json!({"defaultModel": "one", "defaultProvider": "two"})
    );
    assert!(d.agent.join("settings.json.lock").exists());
    // `a` does not see `b`'s write until reload.
    assert_eq!(a.settings().default_provider(), None);
    a.reload();
    assert_eq!(a.settings().default_provider(), Some("two"));
}

#[test]
fn concurrent_threads_do_not_lose_writes() {
    let d = dirs();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let (cwd, agent) = (d.cwd.clone(), d.agent.clone());
            std::thread::spawn(move || {
                let mut sm = SettingsManager::create(&cwd, &agent, true);
                sm.set(SettingsScope::Global, &[&format!("k{i}")], Some(json!(i)))
                    .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let file = read_json(&d.agent.join("settings.json"));
    assert_eq!(file.as_object().unwrap().len(), 8);
}

#[test]
fn delete_with_none_and_nested_paths() {
    let d = dirs();
    write_json(
        &d.agent.join("settings.json"),
        &json!({"shellPath": "/bin/zsh", "images": {"autoResize": false, "x": 1}}),
    );
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, true);
    sm.set_shell_path(None).unwrap();
    sm.set(SettingsScope::Global, &["images", "x"], None)
        .unwrap();
    assert_eq!(
        read_json(&d.agent.join("settings.json")),
        json!({"images": {"autoResize": false}})
    );
    assert_eq!(sm.settings().shell_path(), None);
}

#[test]
fn trust_switching_loads_and_clears_project_layer() {
    let d = dirs();
    write_json(
        &d.agent.join("settings.json"),
        &json!({"defaultModel": "g"}),
    );
    write_json(
        &d.cwd.join(".oino/settings.json"),
        &json!({"defaultModel": "p"}),
    );
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, false);
    assert_eq!(sm.settings().default_model(), Some("g"));
    let err = sm
        .set_resource_paths(SettingsScope::Project, ResourceKind::Skills, &["s".into()])
        .unwrap_err();
    assert_eq!(err.kind, SettingsErrorKind::ProjectUntrusted);
    sm.set_project_trusted(true);
    assert_eq!(sm.settings().default_model(), Some("p"));
    sm.set_resource_paths(SettingsScope::Project, ResourceKind::Skills, &["s".into()])
        .unwrap();
    assert_eq!(
        sm.settings()
            .resource_entries(ResourceKind::Skills, SettingsScope::Project),
        vec!["s".to_owned()]
    );
    sm.set_project_trusted(false);
    assert_eq!(sm.settings().project().len(), 0);
}

#[test]
fn reload_keeps_previous_layer_on_parse_error() {
    let d = dirs();
    let file = d.agent.join("settings.json");
    write_json(&file, &json!({"defaultModel": "a"}));
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, true);
    fs::write(&file, "{ broken").unwrap();
    sm.reload();
    assert_eq!(sm.settings().default_model(), Some("a"));
    let errors = sm.drain_errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].to_string().starts_with("Invalid settings file "));
    assert_eq!(sm.drain_errors().len(), 0);
    // A failed layer is never overwritten.
    sm.set_default_model("b").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "{ broken");
    assert_eq!(sm.settings().default_model(), Some("b"));
}

#[test]
fn not_an_object_is_an_error() {
    let d = dirs();
    fs::create_dir_all(&d.agent).unwrap();
    fs::write(d.agent.join("settings.json"), "[1]").unwrap();
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, true);
    let errors = sm.drain_errors();
    assert_eq!(errors[0].kind, SettingsErrorKind::NotAnObject);
    assert_eq!(errors[0].scope, SettingsScope::Global);
}

#[test]
fn extension_settings_layer_defaults_global_project() {
    let d = dirs();
    write_json(
        &d.agent.join("settings.json"),
        &json!({"extensionSettings": {"ns": {"a": 1, "b": {"x": 1}}}}),
    );
    write_json(
        &d.cwd.join(".oino/settings.json"),
        &json!({"extensionSettings": {"ns": {"b": {"y": 2}}}}),
    );
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, true);
    let defaults = json!({"a": 0, "c": 3});
    assert_eq!(
        sm.settings().extension_settings("ns", Some(&defaults)),
        json!({"a": 1, "c": 3, "b": {"x": 1, "y": 2}})
    );
    sm.set_extension_setting(SettingsScope::Project, "ns", "a", Some(json!(9)))
        .unwrap();
    assert_eq!(sm.settings().extension_settings("ns", None)["a"], json!(9));
    // "use default": delete the project key, global value shows through again.
    sm.set_extension_setting(SettingsScope::Project, "ns", "a", None)
        .unwrap();
    assert_eq!(sm.settings().extension_settings("ns", None)["a"], json!(1));
    assert_eq!(
        read_json(&d.cwd.join(".oino/settings.json")),
        json!({"extensionSettings": {"ns": {"b": {"y": 2}}}})
    );
}

#[test]
fn oino_key_defaults_and_setters() {
    let mut sm = SettingsManager::in_memory(serde_json::Map::new());
    let s = sm.settings();
    assert_eq!(s.serve_settings().bind, "127.0.0.1:7420");
    assert!(s.serve_settings().tls);
    assert!(!s.serve_settings().allow_remote);
    assert_eq!(s.ui_density(), UiDensity::Regular);
    assert_eq!(s.disabled_extensions().len(), 0);
    sm.set_ui_density(UiDensity::Advanced).unwrap();
    sm.set_disabled_extensions(&["x".into()]).unwrap();
    sm.set_default_project_trust(ProjectTrustDefault::Always)
        .unwrap();
    let s = sm.settings();
    assert_eq!(s.ui_density(), UiDensity::Advanced);
    assert_eq!(s.disabled_extensions(), vec!["x".to_owned()]);
    assert_eq!(s.default_project_trust(), ProjectTrustDefault::Always);
}

#[test]
fn device_id_is_created_once() {
    let d = dirs();
    let mut sm = SettingsManager::create(&d.cwd, &d.agent, true);
    let id = sm.get_or_create_device_id().unwrap();
    assert_eq!(sm.get_or_create_device_id().unwrap(), id);
    let reopened = SettingsManager::create(&d.cwd, &d.agent, true);
    assert_eq!(reopened.settings().device_id(), Some(id.as_str()));
}

#[test]
fn overrides_survive_writes() {
    let mut sm = SettingsManager::in_memory(serde_json::Map::new());
    let overrides = json!({"defaultModel": "o"});
    sm.apply_overrides(overrides.as_object().unwrap());
    sm.set_default_model("written").unwrap();
    assert_eq!(sm.settings().default_model(), Some("o"));
    assert_eq!(sm.settings().global()["defaultModel"], json!("written"));
}

#[test]
fn in_memory_migrates_initial_settings() {
    let initial = json!({"queueMode": "all"});
    let sm = SettingsManager::in_memory(initial.as_object().unwrap().clone());
    assert_eq!(sm.settings().steering_mode(), oino_settings::QueueMode::All);
    assert_eq!(sm.path(SettingsScope::Global), None);
}
