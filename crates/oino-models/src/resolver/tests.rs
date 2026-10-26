//! Initial selection, restore and cycling tests (ported from pi behaviour).

use indexmap::IndexMap;
use oino_ai::ThinkingLevel;
use serde_json::json;

use super::*;

struct Catalog {
    all: Vec<Model>,
    authed: Vec<&'static str>,
}

impl ModelCatalog for Catalog {
    fn all(&self) -> Vec<Model> {
        self.all.clone()
    }

    fn available(&self) -> Vec<Model> {
        self.all
            .iter()
            .filter(|m| self.authed.contains(&m.provider.as_str()))
            .cloned()
            .collect()
    }

    fn find(&self, provider: &str, id: &str) -> Option<Model> {
        self.all
            .iter()
            .find(|m| m.provider == provider && m.id == id)
            .cloned()
    }

    fn has_configured_auth(&self, provider: &str) -> bool {
        self.authed.contains(&provider)
    }
}

fn model(provider: &str, id: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": "x", "provider": provider, "baseUrl": "",
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "reasoning": true, "contextWindow": 1, "maxTokens": 1
    }))
    .unwrap()
}

fn catalog(authed: Vec<&'static str>) -> Catalog {
    Catalog {
        all: vec![
            model("local", "first"),
            model("openai", "gpt-5.5"),
            model("openai", "other"),
            model("anthropic", "claude-opus-4-8"),
        ],
        authed,
    }
}

#[test]
fn initial_model_priority() {
    let levels: IndexMap<String, ThinkingLevel> =
        [("openai/other".to_owned(), ThinkingLevel::Low)].into();
    let scoped = vec![ScopedModel {
        model: model("openai", "other"),
        thinking_level: None,
    }];
    let options = InitialModelOptions {
        scoped_models: &scoped,
        model_thinking_levels: Some(&levels),
        ..InitialModelOptions::default()
    };
    let initial = find_initial_model(&options, &catalog(vec![]));
    assert_eq!(initial.model.unwrap().id, "other");
    assert_eq!(initial.thinking_level, ThinkingLevel::Low);

    // Continuing ignores the scope; the settings default needs auth.
    let options = InitialModelOptions {
        scoped_models: &scoped,
        is_continuing: true,
        default_provider: Some("openai"),
        default_model_id: Some("other"),
        default_thinking_level: Some(ThinkingLevel::High),
        ..InitialModelOptions::default()
    };
    let initial = find_initial_model(&options, &catalog(vec!["openai"]));
    assert_eq!(initial.model.unwrap().id, "other");
    assert_eq!(initial.thinking_level, ThinkingLevel::High);
    let initial = find_initial_model(&options, &catalog(vec!["local"]));
    assert_eq!(initial.model.unwrap().id, "first");
    assert_eq!(initial.thinking_level, DEFAULT_THINKING_LEVEL);

    // Known provider defaults win over catalog order.
    let initial = find_initial_model(
        &InitialModelOptions::default(),
        &catalog(vec!["local", "anthropic", "openai"]),
    );
    assert_eq!(initial.model.unwrap().id, "claude-opus-4-8");
    assert_eq!(
        find_initial_model(&InitialModelOptions::default(), &catalog(vec![])).model,
        None
    );
}

#[test]
fn restore_texts() {
    let restored = restore_model_from_session("openai", "gpt-5.5", None, &catalog(vec!["openai"]));
    assert_eq!(restored.model.unwrap().id, "gpt-5.5");
    assert_eq!(
        restored.notices,
        vec![RestoreNotice::Info("Restored model: openai/gpt-5.5".into())]
    );

    let current = model("local", "first");
    let restored =
        restore_model_from_session("openai", "gpt-5.5", Some(&current), &catalog(vec![]));
    assert_eq!(
        restored.fallback_message.as_deref(),
        Some("Could not restore model openai/gpt-5.5 (no auth configured). Using local/first.")
    );

    let restored = restore_model_from_session("gone", "x", None, &catalog(vec!["openai"]));
    assert_eq!(
        restored.fallback_message.as_deref(),
        Some("Could not restore model gone/x (model no longer exists). Using openai/gpt-5.5.")
    );
    assert_eq!(
        restored.notices,
        vec![
            RestoreNotice::Warning(
                "Warning: Could not restore model gone/x (model no longer exists).".into()
            ),
            RestoreNotice::Info("Falling back to: openai/gpt-5.5".into()),
        ]
    );
    let restored = restore_model_from_session("gone", "x", None, &catalog(vec![]));
    assert_eq!((restored.model, restored.fallback_message), (None, None));
}

#[test]
fn cycling() {
    let available = catalog(vec!["openai", "local"]).available();
    let current = model("openai", "other");
    assert_eq!(
        next_available_model(&available, Some(&current), CycleDirection::Forward)
            .unwrap()
            .id,
        "first"
    );
    assert_eq!(
        next_available_model(&available, Some(&current), CycleDirection::Backward)
            .unwrap()
            .id,
        "gpt-5.5"
    );
    let scoped = vec![
        ScopedModel {
            model: model("openai", "gpt-5.5"),
            thinking_level: Some(ThinkingLevel::High),
        },
        ScopedModel {
            model: model("anthropic", "claude-opus-4-8"),
            thinking_level: None,
        },
        ScopedModel {
            model: model("local", "first"),
            thinking_level: None,
        },
    ];
    // anthropic is not available, so the cycle is gpt-5.5 <-> first.
    let next = next_scoped_model(
        &scoped,
        &available,
        Some(&model("local", "first")),
        CycleDirection::Forward,
    )
    .unwrap();
    assert_eq!(next.model.id, "gpt-5.5");
    assert_eq!(next.thinking_level, Some(ThinkingLevel::High));
    assert!(next_scoped_model(&scoped[..2], &available, None, CycleDirection::Forward).is_none());
}

#[test]
fn thinking_level_on_switch() {
    let levels: IndexMap<String, ThinkingLevel> =
        [("openai/gpt-5.5".to_owned(), ThinkingLevel::Minimal)].into();
    let target = model("openai", "gpt-5.5");
    let inputs = ThinkingSwitch {
        model_thinking_levels: Some(&levels),
        default_thinking_level: Some(ThinkingLevel::High),
        current: Some(ThinkingLevel::Low),
        ..ThinkingSwitch::default()
    };
    assert_eq!(
        thinking_level_for_model_switch(Some(&target), &inputs),
        ThinkingLevel::Minimal
    );
    let other = model("openai", "other");
    assert_eq!(
        thinking_level_for_model_switch(Some(&other), &inputs),
        ThinkingLevel::High
    );
    let explicit = ThinkingSwitch {
        explicit: Some(ThinkingLevel::Off),
        ..inputs
    };
    assert_eq!(
        thinking_level_for_model_switch(Some(&target), &explicit),
        ThinkingLevel::Off
    );
    assert_eq!(
        thinking_level_for_model_switch(None, &ThinkingSwitch::default()),
        ThinkingLevel::Medium
    );
}

#[test]
fn exact_reference_and_catalog_wrappers() {
    let catalog = catalog(vec!["openai"]);
    let all = catalog.all();
    assert_eq!(
        find_exact_model_reference_match("openai/gpt-5.5", &all).map(|m| m.id.as_str()),
        Some("gpt-5.5")
    );
    assert!(find_exact_model_reference_match("gpt", &all).is_none());
    let scope = resolve_model_scope_in(&["*".to_owned()], &catalog);
    assert_eq!(scope.scoped_models.len(), 2);
    let cli = resolve_cli_model_in(
        &CliModelRequest {
            model: Some("opus"),
            ..CliModelRequest::default()
        },
        &catalog,
    )
    .unwrap()
    .unwrap();
    assert_eq!(cli.model.id, "claude-opus-4-8");
}
