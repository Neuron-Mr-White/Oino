//! Golden tests against pi 1.4.0 (fixtures from `scripts/pi-oracle/core-runtime/models`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::Path;
use std::sync::Arc;

use indexmap::IndexMap;
use oino_ai::json::json_semantic_eq;
use oino_ai::{Model, ThinkingLevel};
use oino_models::config_value::{
    config_value_env_var_name, config_value_env_var_names, is_config_value_configured,
    resolve_config_value_strict, resolve_headers, resolve_headers_strict,
};
use oino_models::resolver::{
    CliModelRequest, ModelReferenceRequest, ModelReferenceResult, ThinkingSuffix,
    resolve_cli_model, resolve_model_reference, resolve_model_scope,
};
use oino_models::virtual_models::{VirtualModelDefinition, create_virtual_model};
use oino_models::{
    AuthCredential, AuthOverrides, AuthStorage, MapEnv, ModelRegistry, ModelRegistryOptions,
    ModelsJson, ProviderConfigOverride, ProviderModelConfig, ProviderModels, ProviderRegistration,
    resolve_config_value,
};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn opt_str(value: &Value) -> Option<&str> {
    value.as_str()
}

fn key(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

fn level(value: Option<ThinkingLevel>) -> Value {
    value.map_or(Value::Null, |l| json!(l.as_str()))
}

fn summarize(result: &ModelReferenceResult<'_>) -> Value {
    match result {
        ModelReferenceResult::Resolved {
            model,
            thinking_level,
            warning,
        } => json!({
            "status": "resolved", "model": key(model), "thinkingLevel": level(*thinking_level), "warning": warning,
        }),
        ModelReferenceResult::Ambiguous {
            candidates,
            usable_count,
            thinking_level,
            warning,
        } => json!({
            "status": "ambiguous", "candidates": candidates.iter().map(|m| key(m)).collect::<Vec<_>>(),
            "usableCount": usable_count, "thinkingLevel": level(*thinking_level), "warning": warning,
        }),
        ModelReferenceResult::NotFound { target } => json!({
            "status": "not-found",
            "target": target.as_ref().map(|t| json!({
                "provider": t.provider,
                "providerModels": t.provider_models.iter().map(|m| key(m)).collect::<Vec<_>>(),
                "modelText": t.model_text, "modelId": t.model_id, "thinkingLevel": level(t.thinking_level),
            })),
        }),
    }
}

fn suffix(value: &str) -> ThinkingSuffix {
    match value {
        "none" => ThinkingSuffix::None,
        "strict" => ThinkingSuffix::Strict,
        _ => ThinkingSuffix::Lenient,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn resolve_model_reference_matches_pi() {
    let data = fixture("resolver.json");
    let catalog: Vec<Model> = serde_json::from_value(data["catalog"].clone()).unwrap();
    let cases = data["referenceCases"].as_array().unwrap();
    let mut failures = Vec::new();
    for case in cases {
        let request = &case["request"];
        let usable = strings(&request["usable"]);
        let result = resolve_model_reference(
            &ModelReferenceRequest {
                text: request["text"].as_str().unwrap(),
                provider: opt_str(&request["provider"]),
                thinking_suffix: suffix(request["thinkingSuffix"].as_str().unwrap()),
                fuzzy: request["fuzzy"].as_bool().unwrap(),
            },
            &catalog,
            &|p| usable.iter().any(|u| u == p),
        );
        let actual = summarize(&result);
        if actual != case["result"] {
            failures.push(format!(
                "{request}\n  expected {}\n  actual   {actual}",
                case["result"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(cases.len() > 1000);
}

#[test]
fn resolve_model_scope_matches_pi() {
    let data = fixture("resolver.json");
    let catalog: Vec<Model> = serde_json::from_value(data["catalog"].clone()).unwrap();
    for case in data["scopeCases"].as_array().unwrap() {
        let patterns = strings(&case["patterns"]);
        let result = resolve_model_scope(&patterns, &catalog);
        let scoped: Vec<Value> = result
            .scoped_models
            .iter()
            .map(|s| json!({"model": key(&s.model), "thinkingLevel": level(s.thinking_level)}))
            .collect();
        assert_eq!(json!(scoped), case["scopedModels"], "patterns {patterns:?}");
        let diagnostics = serde_json::to_value(&result.diagnostics).unwrap();
        assert_eq!(diagnostics, case["diagnostics"], "patterns {patterns:?}");
    }
}

#[test]
fn resolve_cli_model_matches_pi() {
    let data = fixture("resolver.json");
    let catalog: Vec<Model> = serde_json::from_value(data["catalog"].clone()).unwrap();
    for case in data["cliCases"].as_array().unwrap() {
        let input = &case["input"];
        let authed = strings(&input["authed"]);
        let models = if input["emptyCatalog"].as_bool() == Some(true) {
            Vec::new()
        } else {
            catalog.clone()
        };
        let request = CliModelRequest {
            provider: opt_str(&input["cliProvider"]),
            model: opt_str(&input["cliModel"]),
            thinking: opt_str(&input["cliThinking"]).map(|l| l.parse().unwrap()),
            inline_api_key: input["inlineApiKey"].as_bool().unwrap_or(false),
        };
        let result = resolve_cli_model(&request, &models, &|p| authed.iter().any(|a| a == p));
        match result {
            Ok(Some(resolved)) => {
                let model = serde_json::to_value(&resolved.model).unwrap();
                assert!(
                    json_semantic_eq(&model, &case["model"]),
                    "{input}: {model} != {}",
                    case["model"]
                );
                assert_eq!(
                    level(resolved.thinking_level),
                    case["thinkingLevel"],
                    "{input}"
                );
                assert_eq!(json!(resolved.warning), case["warning"], "{input}");
                assert!(case["error"].is_null(), "{input}");
            }
            Ok(None) => assert!(
                case["model"].is_null() && case["error"].is_null(),
                "{input}"
            ),
            Err(error) => assert_eq!(json!(error.0), case["error"], "{input}"),
        }
    }
}

#[test]
fn config_values_match_pi() {
    let data = fixture("config-value.json");
    let env: MapEnv = data["env"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect();
    for case in data["cases"].as_array().unwrap() {
        let config = case["config"].as_str().unwrap();
        assert_eq!(
            json!(resolve_config_value(config, None, &env)),
            case["resolved"],
            "{config}"
        );
        assert_eq!(
            json!(config_value_env_var_name(config)),
            case["envVarName"],
            "{config}"
        );
        assert_eq!(
            json!(config_value_env_var_names(config)),
            case["envVarNames"],
            "{config}"
        );
        assert_eq!(
            json!(is_config_value_configured(config, None, &env)),
            case["configured"],
            "{config}"
        );
        let strict = match resolve_config_value_strict(config, "API key", None, &env) {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"error": error.to_string()}),
        };
        assert_eq!(strict, case["strict"], "{config}");
    }
    for case in data["explicitCases"].as_array().unwrap() {
        let explicit: IndexMap<String, String> =
            serde_json::from_value(case["explicit"].clone()).unwrap();
        let config = case["config"].as_str().unwrap();
        assert_eq!(
            json!(resolve_config_value(config, Some(&explicit), &env)),
            case["resolved"],
            "{config}"
        );
    }
    let headers: IndexMap<String, String> =
        serde_json::from_value(data["headers"].clone()).unwrap();
    assert_eq!(
        json!(resolve_headers(Some(&headers), None, &env)),
        data["resolvedHeaders"]
    );
    let strict = match resolve_headers_strict(Some(&headers), "provider \"p\"", None, &env) {
        Ok(value) => json!({"ok": value}),
        Err(error) => json!({"error": error.to_string()}),
    };
    assert_eq!(strict, data["strictHeaders"]);
}

#[test]
fn models_json_diagnostics_match_pi() {
    let data = fixture("models-json.json");
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let config = ModelsJson::parse_at(case["content"].as_str().unwrap(), Path::new("<FILE>"));
        assert_eq!(json!(config.error()), case["error"], "{name}");
        let ids: Vec<&str> = config.provider_ids().collect();
        assert_eq!(json!(ids), case["providerIds"], "{name}");
    }
}

fn registry_for(base: Option<&Value>, env: MapEnv, credential: Option<&Value>) -> ModelRegistry {
    let env = Arc::new(env);
    let mut stored = IndexMap::new();
    if let Some(credential) = credential {
        stored.insert(
            "p".to_owned(),
            AuthCredential::from_value(credential).unwrap(),
        );
    }
    let auth = Arc::new(AuthStorage::in_memory(stored).with_env(env.clone()));
    let registry = ModelRegistry::new(
        auth,
        ModelRegistryOptions {
            env,
            ..ModelRegistryOptions::default()
        },
    );
    if let Some(base) = base {
        let models: Vec<ProviderModelConfig> = serde_json::from_value(base.clone()).unwrap();
        registry
            .register_provider(
                "ext.p",
                ProviderRegistration {
                    id: "p".to_owned(),
                    name: Some("Base P".to_owned()),
                    api: "openai-completions".to_owned(),
                    base_url: Some("https://p.example/v1".to_owned()),
                    auth_env: vec!["P_KEY".to_owned()],
                    auth_header: oino_models::AuthHeaderMode::None,
                    oauth: false,
                    headers: IndexMap::new(),
                    compat: None,
                    models: ProviderModels::Static(models),
                },
            )
            .unwrap();
    }
    registry
}

fn models_json_for(provider: &Value) -> ModelsJson {
    let content = if provider.is_null() {
        json!({"providers": {}})
    } else {
        json!({"providers": {"p": provider}})
    };
    let config = ModelsJson::parse_at(&content.to_string(), Path::new("<FILE>"));
    assert_eq!(config.error(), None);
    config
}

#[test]
fn composition_matches_pi() {
    let data = fixture("compose.json");
    for case in data["composeCases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let registry = registry_for(
            (!case["base"].is_null()).then_some(&case["base"]),
            MapEnv::new(),
            None,
        );
        registry.set_models_json(models_json_for(&case["modelsJson"]));
        if !case["extension"].is_null() {
            let config: ProviderConfigOverride =
                serde_json::from_value(case["extension"].clone()).unwrap();
            registry.register_provider_config("p", config).unwrap();
        }
        if let Some(error) = case["error"].as_str() {
            assert_eq!(
                registry.error(),
                Some(format!("Provider \"p\": {error}")),
                "{name}"
            );
            continue;
        }
        assert_eq!(registry.error(), None, "{name}");
        let actual = serde_json::to_value(registry.all()).unwrap();
        assert!(
            json_semantic_eq(&actual, &case["models"]),
            "{name}:\n  actual   {actual}\n  expected {}",
            case["models"]
        );
    }
}

#[test]
fn auth_resolution_matches_pi() {
    let data = fixture("compose.json");
    for case in data["authCases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let mut env = MapEnv::new();
        for source in [&input["env"], &input["envExtra"]] {
            for (k, v) in source.as_object().into_iter().flatten() {
                env = env.with(k.clone(), v.as_str().unwrap());
            }
        }
        let base = json!([{"id": "base-a", "name": "base-a"}]);
        let registry = if input["noBase"].as_bool() == Some(true) {
            registry_for(None, env, input.get("credential"))
        } else {
            let registry = registry_for(Some(&base), env.clone(), input.get("credential"));
            // Re-register with the case's env vars / OAuth flag.
            let mut registration = registry.registration("p").unwrap();
            registration.auth_env = match &input["envVars"] {
                Value::Null if input.get("envVars").is_some() => Vec::new(),
                Value::Array(_) => strings(&input["envVars"]),
                _ => vec!["P_KEY".to_owned()],
            };
            registration.oauth = input["oauth"].as_bool().unwrap_or(false);
            registry.register_provider("ext.p", registration).unwrap();
            registry
        };
        registry.set_models_json(models_json_for(&input["modelsJson"]));
        if !input["extension"].is_null() {
            let config: ProviderConfigOverride =
                serde_json::from_value(input["extension"].clone()).unwrap();
            registry.register_provider_config("p", config).unwrap();
        }
        let check = registry
            .check_auth("p")
            .map(|c| json!({"type": c.kind.as_str(), "source": c.source}));
        assert_eq!(json!(check), case["check"], "{name} check");
        let resolved =
            futures::executor::block_on(registry.get_auth("p", &AuthOverrides::default()));
        match resolved {
            Ok(resolved) => {
                assert!(
                    case["error"].is_null(),
                    "{name}: expected error {}",
                    case["error"]
                );
                let actual = resolved.map(
                    |r| json!({"apiKey": r.api_key, "headers": r.headers, "source": r.source}),
                );
                assert_eq!(json!(actual), case["resolved"], "{name} resolve");
            }
            Err(error) => assert_eq!(
                json!(error.request_message("p")),
                case["error"],
                "{name} error"
            ),
        }
    }
}

#[test]
fn virtual_models_match_pi() {
    let data = fixture("virtual-models.json");
    assert_eq!(
        data["api"],
        oino_models::VIRTUAL_MODEL_API.replace("oino", "pi")
    );
    assert_eq!(
        data["stateEntry"],
        oino_models::virtual_models::VIRTUAL_MODEL_STATE_ENTRY.replace("oino", "pi")
    );
    for case in data["models"].as_array().unwrap() {
        let definition: VirtualModelDefinition =
            serde_json::from_value(case["definition"].clone()).unwrap();
        let mut expected = case["model"].clone();
        expected["api"] = json!(oino_models::VIRTUAL_MODEL_API);
        let actual = serde_json::to_value(create_virtual_model(&definition)).unwrap();
        assert!(
            json_semantic_eq(&actual, &expected),
            "{actual} != {expected}"
        );
    }
}
