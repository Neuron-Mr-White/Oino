//! `notify` settings (unipi `settings.ts`, `ntfy-config.ts`, `types.ts`).
//!
//! Values are `defaults ⊕ host settings ⊕ local overlay` (the overlay holds what
//! `/unipi:notify-event` and the setup wizards write until the host offers a guest settings
//! write, unipi-core R1). Secret fields (`gotify.appToken`, `telegram.botToken`,
//! `telegram.chatId`, `ntfy.token`) come from the settings value when present, else the
//! extension's secret store (`notify/<key>`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use unipi_kit::settings::{Choice, Field, Kind, Namespace, Section, Unit, plain};

/// Extension id.
pub(crate) const EXTENSION_ID: &str = "unipi.notify";
/// Settings namespace.
pub(crate) const NAMESPACE: &str = "notify";

/// Known platforms.
pub(crate) const PLATFORMS: [&str; 4] = ["native", "gotify", "telegram", "ntfy"];

/// Known events in unipi order (`DEFAULT_CONFIG.events`).
pub(crate) const EVENTS: [&str; 8] = [
    "ralph_loop_end",
    "mcp_server_error",
    "agent_end",
    "agent_settled",
    "session_shutdown",
    "ask_user_prompt",
    "permission_request",
    "input_needed",
];

/// Secret fields (masked in the hub, stored in the secret store).
pub(crate) const SECRET_KEYS: [&str; 4] = [
    "gotify.appToken",
    "telegram.botToken",
    "telegram.chatId",
    "ntfy.token",
];

/// unipi `EVENT_PHRASES`.
fn event_phrase(event: &str) -> &'static str {
    match event {
        "ralph_loop_end" => "a ralph loop iteration ends.",
        "mcp_server_error" => "an MCP server errors.",
        "agent_end" => {
            "a run finishes (once per run, after every follow-up and continuation, not per message)."
        }
        "agent_settled" => {
            "a run finishes (same single notification as agent_end; either switch turns it on)."
        }
        "session_shutdown" => "the session shuts down.",
        "ask_user_prompt" => "the agent asks you a question.",
        "permission_request" => "a permission prompt opens.",
        "input_needed" => {
            "the agent is waiting on any prompt (covers tools that send no event of their own)."
        }
        _ => "the event fires.",
    }
}

/// Per-event routing (unipi `EventNotifyConfig`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EventConfig {
    /// Enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Platforms (empty = all enabled platforms, then defaults).
    #[serde(default)]
    pub platforms: Vec<String>,
}

/// unipi `NativeConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeConfig {
    /// Enabled.
    pub enabled: bool,
    /// Hold native notifications while a frontend has focus.
    #[serde(default)]
    pub suppress_when_focused: bool,
}

/// unipi `GotifyConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GotifyConfig {
    /// Enabled.
    pub enabled: bool,
    /// Server URL.
    #[serde(default)]
    pub server_url: Option<String>,
    /// App token.
    #[serde(default)]
    pub app_token: Option<String>,
    /// Priority 0–10.
    pub priority: i64,
}

/// unipi `TelegramConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TelegramConfig {
    /// Enabled.
    pub enabled: bool,
    /// Bot token.
    #[serde(default)]
    pub bot_token: Option<String>,
    /// Chat id.
    #[serde(default)]
    pub chat_id: Option<String>,
}

/// unipi `NtfyConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NtfyConfig {
    /// Enabled.
    pub enabled: bool,
    /// Server (default `https://ntfy.sh`).
    #[serde(default)]
    pub server_url: Option<String>,
    /// Topic.
    #[serde(default)]
    pub topic: Option<String>,
    /// Access token.
    #[serde(default)]
    pub token: Option<String>,
    /// Priority 1–5.
    pub priority: i64,
    /// `minimal` (default) | `full`.
    #[serde(default)]
    pub app_detail: Option<String>,
}

/// unipi `RecapConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecapConfig {
    /// Enabled.
    pub enabled: bool,
    /// `provider/model` (empty = session model).
    pub model: String,
    /// Ask the server to skip reasoning.
    #[serde(default)]
    pub disable_thinking: bool,
}

/// unipi `SilenceAfterInputConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SilenceConfig {
    /// Master switch.
    pub enabled: bool,
    /// Quiet window (ms).
    pub window_ms: u64,
    /// Platforms to quiet (empty = all).
    pub platforms: Vec<String>,
}

/// unipi `RenotifyConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenotifyConfig {
    /// Master switch.
    pub enabled: bool,
    /// Delay between reminders (ms, ≥ 10 000).
    pub interval_ms: u64,
    /// Reminders after the first notification.
    pub max_repeats: u64,
}

/// unipi `NotifyConfig` (+ the ntfy subtree, which lives in the same namespace).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotifyConfig {
    /// Fallback platforms.
    pub default_platforms: Vec<String>,
    /// Events in [`EVENTS`] order.
    pub events: Vec<(String, EventConfig)>,
    /// Native.
    pub native: NativeConfig,
    /// Gotify.
    pub gotify: GotifyConfig,
    /// Telegram.
    pub telegram: TelegramConfig,
    /// ntfy.
    pub ntfy: NtfyConfig,
    /// Recap.
    pub recap: RecapConfig,
    /// Silence after input.
    pub silence_after_input: SilenceConfig,
    /// Renotify.
    pub renotify: RenotifyConfig,
}

impl NotifyConfig {
    /// Routing of `event` (`None` for unknown events).
    pub(crate) fn event(&self, event: &str) -> Option<&EventConfig> {
        self.events.iter().find(|(k, _)| k == event).map(|(_, c)| c)
    }

    /// Whether `platform` is enabled.
    pub(crate) fn platform_enabled(&self, platform: &str) -> bool {
        match platform {
            "native" => self.native.enabled,
            "gotify" => self.gotify.enabled,
            "telegram" => self.telegram.enabled,
            "ntfy" => self.ntfy.enabled,
            _ => false,
        }
    }

    /// unipi `getEnabledPlatforms`.
    pub(crate) fn enabled_platforms(&self) -> Vec<String> {
        PLATFORMS
            .iter()
            .filter(|p| self.platform_enabled(p))
            .map(|p| (*p).to_owned())
            .collect()
    }

    /// unipi `dispatchNotification` platform resolution: event platforms → every enabled
    /// platform → defaults, then only enabled ones.
    pub(crate) fn resolve_platforms(&self, event_platforms: &[String]) -> Vec<String> {
        let enabled = self.enabled_platforms();
        let chosen: Vec<String> = if !event_platforms.is_empty() {
            event_platforms.to_vec()
        } else if !enabled.is_empty() {
            enabled
        } else {
            self.default_platforms.clone()
        };
        chosen
            .into_iter()
            .filter(|p| self.platform_enabled(p))
            .collect()
    }

    /// unipi `validateConfig`.
    pub(crate) fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.gotify.enabled {
            if self.gotify.server_url.as_deref().is_none_or(str::is_empty) {
                errors.push("Gotify: serverUrl is required".to_owned());
            }
            if self.gotify.app_token.as_deref().is_none_or(str::is_empty) {
                errors.push("Gotify: appToken is required".to_owned());
            }
        }
        if self.telegram.enabled {
            if self.telegram.bot_token.as_deref().is_none_or(str::is_empty) {
                errors.push("Telegram: botToken is required".to_owned());
            }
            if self.telegram.chat_id.as_deref().is_none_or(str::is_empty) {
                errors.push("Telegram: chatId is required".to_owned());
            }
        }
        if !(1..=10).contains(&self.gotify.priority) {
            errors.push("Gotify: priority must be between 1 and 10".to_owned());
        }
        errors
    }
}

/// unipi `DEFAULT_CONFIG` as settings JSON (+ ntfy defaults).
pub(crate) fn defaults() -> Value {
    let mut events = Map::new();
    for e in EVENTS {
        let on = matches!(e, "ralph_loop_end" | "mcp_server_error");
        events.insert(e.to_owned(), json!({"enabled": on, "platforms": []}));
    }
    json!({
        "defaultPlatforms": ["native"],
        "events": events,
        "native": {"enabled": true, "suppressWhenFocused": false},
        "gotify": {"enabled": false, "priority": 5},
        "telegram": {"enabled": false},
        "ntfy": {"enabled": false, "serverUrl": "https://ntfy.sh", "priority": 3},
        "recap": {"enabled": false, "model": "openrouter/openai/gpt-oss-20b", "disableThinking": false},
        "silenceAfterInput": {"enabled": false, "windowMs": 10000, "platforms": ["native"]},
        "renotify": {"enabled": true, "intervalMs": 120_000, "maxRepeats": 3},
    })
}

/// Expands flat dotted keys (`"events.x.enabled": true`) into nested objects, applied over
/// the nested values (the host stores hub writes as dotted keys).
pub(crate) fn expand_dotted(v: &Value) -> Value {
    let Value::Object(map) = v else {
        return v.clone();
    };
    let mut nested = Map::new();
    let mut flat = Vec::new();
    for (k, val) in map {
        if k.contains('.') {
            flat.push((k.clone(), val.clone()));
        } else {
            nested.insert(k.clone(), expand_dotted(val));
        }
    }
    let mut out = Value::Object(nested);
    for (k, val) in flat {
        out = unipi_kit::settings::set_field(&out, &k, val);
    }
    out
}

fn platforms_of(v: Option<&Value>) -> Option<Vec<String>> {
    v.and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .filter(|p| PLATFORMS.contains(p))
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn obj<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

fn bool_or(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn str_opt(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "settings integers are small"
)]
fn int_or(v: &Value, k: &str, d: i64) -> i64 {
    v.get(k)
        .and_then(|n| n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)))
        .unwrap_or(d)
}

/// unipi `mergeWithDefaults` (+ `mergeRenotify`, `mergeSilenceAfterInput`) over a resolved
/// namespace object. Unknown event keys are dropped.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "settings numbers are small and validated"
)]
pub(crate) fn from_values(values: &Value) -> NotifyConfig {
    let v = expand_dotted(values);
    let d = defaults();
    let events = EVENTS
        .iter()
        .map(|e| {
            let loaded = obj(obj(&v, "events"), e);
            let def = obj(obj(&d, "events"), e);
            (
                (*e).to_owned(),
                EventConfig {
                    enabled: bool_or(loaded, "enabled", bool_or(def, "enabled", false)),
                    platforms: platforms_of(loaded.get("platforms")).unwrap_or_default(),
                },
            )
        })
        .collect();
    let native = obj(&v, "native");
    let gotify = obj(&v, "gotify");
    let telegram = obj(&v, "telegram");
    let ntfy = obj(&v, "ntfy");
    let recap = obj(&v, "recap");
    let silence = obj(&v, "silenceAfterInput");
    let renotify = obj(&v, "renotify");
    let window_ms = silence
        .get("windowMs")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map_or(10_000, |n| n as u64);
    let interval_ms = renotify
        .get("intervalMs")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 10_000.0)
        .map_or(120_000, |n| n as u64);
    let max_repeats = renotify
        .get("maxRepeats")
        .and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0 && *n >= 0.0)
        .map_or(3, |n| n as u64);
    NotifyConfig {
        default_platforms: platforms_of(v.get("defaultPlatforms"))
            .unwrap_or_else(|| vec!["native".to_owned()]),
        events,
        native: NativeConfig {
            enabled: bool_or(native, "enabled", true),
            suppress_when_focused: bool_or(native, "suppressWhenFocused", false),
        },
        gotify: GotifyConfig {
            enabled: bool_or(gotify, "enabled", false),
            server_url: str_opt(gotify, "serverUrl"),
            app_token: str_opt(gotify, "appToken"),
            priority: int_or(gotify, "priority", 5),
        },
        telegram: TelegramConfig {
            enabled: bool_or(telegram, "enabled", false),
            bot_token: str_opt(telegram, "botToken"),
            chat_id: str_opt(telegram, "chatId"),
        },
        ntfy: NtfyConfig {
            enabled: bool_or(ntfy, "enabled", false),
            server_url: str_opt(ntfy, "serverUrl").or_else(|| Some("https://ntfy.sh".into())),
            topic: str_opt(ntfy, "topic"),
            token: str_opt(ntfy, "token"),
            priority: int_or(ntfy, "priority", 3),
            app_detail: str_opt(ntfy, "appDetail"),
        },
        recap: RecapConfig {
            enabled: bool_or(recap, "enabled", false),
            model: recap
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("openrouter/openai/gpt-oss-20b")
                .to_owned(),
            disable_thinking: bool_or(recap, "disableThinking", false),
        },
        silence_after_input: SilenceConfig {
            enabled: bool_or(silence, "enabled", false),
            window_ms,
            platforms: platforms_of(silence.get("platforms"))
                .unwrap_or_else(|| vec!["native".to_owned()]),
        },
        renotify: RenotifyConfig {
            enabled: bool_or(renotify, "enabled", true),
            interval_ms,
            max_repeats,
        },
    }
}

fn platform_options() -> Vec<Choice> {
    PLATFORMS.iter().map(|p| plain(p)).collect()
}

fn multiselect(key: &str, label: &str, empty: &str) -> Field {
    Field::multiselect(key, label, platform_options()).empty_label(empty)
}

fn number(key: &str, label: &str, min: Option<f64>, max: Option<f64>, unit: Option<Unit>) -> Field {
    Field::number(key, label).with_kind(|k| {
        if let Kind::Number {
            min: mn,
            max: mx,
            unit: u,
            ..
        } = k
        {
            *mn = min;
            *mx = max;
            *u = unit;
        }
    })
}

/// The `notify` namespace (unipi `registerSettings({namespace: "notify"})`).
#[allow(clippy::too_many_lines, reason = "one declarative schema")]
pub(crate) fn namespace() -> Namespace {
    let event_sections: Vec<Section> = EVENTS
        .iter()
        .map(|e| {
            Section::new(e, e)
                .field(
                    Field::boolean(&format!("events.{e}.enabled"), "Enabled")
                        .description(format!("Send a notification when {}", event_phrase(e))),
                )
                .field(
                    multiselect(
                        &format!("events.{e}.platforms"),
                        "Platforms",
                        "default platforms",
                    )
                    .description("Where this event notifies. Empty uses the default platforms."),
                )
        })
        .collect();
    let page = |key: &str, desc: &str, fields: Vec<Field>| {
        Field::page(key, key, vec![Section::new(key, key).fields(fields)]).description(desc)
    };
    Namespace::new(NAMESPACE, "Notify", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("general", "General")
                .field(
                    multiselect("defaultPlatforms", "Default platforms", "none")
                        .description("Used when an event lists no platforms of its own."),
                )
                .field(
                    Field::boolean("native.enabled", "Native desktop")
                        .description("OS notifications through the desktop notification service."),
                )
                .field(
                    Field::boolean("native.suppressWhenFocused", "Quiet when focused")
                        .description("Hold native notifications while the terminal has focus."),
                )
                .field(
                    Field::boolean("recap.enabled", "Recap")
                        .description("Periodic digests of what happened while you were away."),
                )
                .field(
                    Field::model("recap.model", "Recap model")
                        .with_kind(|k| {
                            if let Kind::Model {
                                filter,
                                empty_option,
                                empty_label,
                                ..
                            } = k
                            {
                                *filter = Some(json!({"capability": "text"}));
                                *empty_option = Some("inherit (session model)".into());
                                *empty_label = Some("inherit (session model)".into());
                            }
                        })
                        .description("Model that writes the recap; empty uses the session model."),
                ),
        )
        .section(
            Section::new("silence", "Silence & renotify")
                .field(
                    Field::boolean("silenceAfterInput.enabled", "Silence after input")
                        .description("Hold notifications while you are typing."),
                )
                .field(
                    number(
                        "silenceAfterInput.windowMs",
                        "Silence window",
                        Some(0.0),
                        None,
                        Some(Unit::Ms),
                    )
                    .description("How long after input notifications stay held."),
                )
                .field(
                    multiselect("silenceAfterInput.platforms", "Silence platforms", "none")
                        .description("Which platforms quiet down while you type."),
                )
                .field(
                    Field::boolean("renotify.enabled", "Renotify")
                        .description("Repeat notifications that stay unresolved."),
                )
                .field(
                    number(
                        "renotify.intervalMs",
                        "Renotify interval",
                        Some(0.0),
                        None,
                        Some(Unit::Ms),
                    )
                    .description("How often an unresolved notification repeats."),
                )
                .field(
                    number("renotify.maxRepeats", "Max repeats", Some(0.0), None, None)
                        .description("Repeats before the notification is left alone."),
                ),
        )
        .section(
            Section::new("events", "Events")
                .description("Event-by-event routing")
                .field(
                    Field::page("events-page", "Events…", event_sections)
                        .description("Turn each event into a notification and pick where it goes."),
                )
                .field(
                    Field::action(
                        "actions.test",
                        "Send test notification",
                        "unipi:notify-test",
                    )
                    .description("Sends on every enabled platform."),
                ),
        )
        .section(
            Section::new("platforms", "Platforms")
                .description("Per-platform credentials and targets")
                .field(page(
                    "gotify",
                    "Self-hosted Gotify push server.",
                    vec![
                        Field::boolean("gotify.enabled", "Enabled")
                            .description("Deliver notifications through Gotify."),
                        Field::string("gotify.serverUrl", "Server URL")
                            .empty_label("https://gotify.example")
                            .description("Base URL of your Gotify server."),
                        Field::secret("gotify.appToken", "App token")
                            .empty_label("unset")
                            .description("Application token from the Gotify web UI."),
                        number("gotify.priority", "Priority", Some(0.0), Some(10.0), None)
                            .description("Delivery priority (0–10)."),
                        Field::action("gotify.setup", "Setup wizard…", "unipi:notify-set-gotify")
                            .description("Guided setup with a test message."),
                    ],
                ))
                .field(page(
                    "telegram",
                    "Telegram bot delivery.",
                    vec![
                        Field::boolean("telegram.enabled", "Enabled")
                            .description("Deliver notifications through a Telegram bot."),
                        Field::secret("telegram.botToken", "Bot token")
                            .empty_label("unset")
                            .description("Token from @BotFather."),
                        Field::secret("telegram.chatId", "Chat ID")
                            .empty_label("unset")
                            .description("Chat the messages are delivered to."),
                        Field::action("telegram.setup", "Setup wizard…", "unipi:notify-set-tg")
                            .description(
                                "Paste the bot token, message the bot, and the chat id is found.",
                            ),
                    ],
                ))
                .field(page(
                    "ntfy",
                    "ntfy.sh push topics.",
                    vec![
                        Field::boolean("ntfy.enabled", "Enabled")
                            .description("Deliver notifications by publishing to ntfy."),
                        Field::string("ntfy.serverUrl", "Server")
                            .empty_label("https://ntfy.sh")
                            .description("ntfy server to publish to."),
                        Field::string("ntfy.topic", "Topic")
                            .empty_label("unset")
                            .description("Topic name your devices subscribe to."),
                        Field::secret("ntfy.token", "Access token")
                            .empty_label("unset")
                            .description("Token for protected topics."),
                        number("ntfy.priority", "Priority", Some(1.0), Some(5.0), None)
                            .description("Delivery priority (1–5)."),
                        Field::enumeration(
                            "ntfy.appDetail",
                            "Message detail",
                            vec![
                                unipi_kit::settings::choice(
                                    "minimal",
                                    "minimal",
                                    "generic text; open Oino for details",
                                ),
                                unipi_kit::settings::choice(
                                    "full",
                                    "full",
                                    "the full notification text",
                                ),
                            ],
                        )
                        .description(
                            "How much of the notification text rides along in the ntfy payload.",
                        ),
                        Field::action("ntfy.setup", "Setup wizard…", "unipi:notify-set-ntfy")
                            .description("Guided setup with a test message."),
                    ],
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_unipi() {
        let c = from_values(&defaults());
        assert_eq!(c.default_platforms, ["native"]);
        assert_eq!(c.events.len(), 8);
        assert!(c.event("ralph_loop_end").is_some_and(|e| e.enabled));
        assert!(c.event("mcp_server_error").is_some_and(|e| e.enabled));
        assert!(c.event("agent_end").is_some_and(|e| !e.enabled));
        assert!(c.event("input_needed").is_some_and(|e| !e.enabled));
        assert!(c.native.enabled);
        assert_eq!(c.gotify.priority, 5);
        assert_eq!(c.ntfy.priority, 3);
        assert_eq!(c.ntfy.server_url.as_deref(), Some("https://ntfy.sh"));
        assert_eq!(c.recap.model, "openrouter/openai/gpt-oss-20b");
        assert_eq!(c.silence_after_input.window_ms, 10_000);
        assert_eq!(c.silence_after_input.platforms, ["native"]);
        assert_eq!(
            c.renotify,
            RenotifyConfig {
                enabled: true,
                interval_ms: 120_000,
                max_repeats: 3
            }
        );
        assert_eq!(c.enabled_platforms(), ["native"]);
    }

    #[test]
    fn merge_rules() {
        let v = json!({
            "events": {"agent_end": {"enabled": true, "platforms": ["ntfy", "bogus"]}, "old_event": {"enabled": true}},
            "renotify": {"intervalMs": 5000, "maxRepeats": 1.5},
            "silenceAfterInput": {"windowMs": -1, "platforms": ["gotify", "nope"]},
            "gotify": {"enabled": true, "serverUrl": "https://g", "priority": 8},
        });
        let c = from_values(&v);
        assert!(c.event("old_event").is_none(), "unknown events dropped");
        let ae = c.event("agent_end").cloned().unwrap_or_default();
        assert!(ae.enabled);
        assert_eq!(ae.platforms, ["ntfy"], "unknown platforms dropped");
        assert_eq!(c.renotify.interval_ms, 120_000, "below 10 s falls back");
        assert_eq!(c.renotify.max_repeats, 3, "non-integer falls back");
        assert_eq!(c.silence_after_input.window_ms, 10_000);
        assert_eq!(c.silence_after_input.platforms, ["gotify"]);
        assert_eq!(c.validate(), ["Gotify: appToken is required"]);
    }

    #[test]
    fn dotted_keys_are_expanded() {
        let v = json!({"events": {"agent_end": {"enabled": false}}, "events.agent_end.enabled": true, "native.enabled": false});
        let c = from_values(&v);
        assert!(c.event("agent_end").is_some_and(|e| e.enabled));
        assert!(!c.native.enabled);
    }

    #[test]
    fn platform_resolution() {
        let mut c = from_values(&defaults());
        assert_eq!(c.resolve_platforms(&[]), ["native"]);
        c.gotify.enabled = true;
        assert_eq!(
            c.resolve_platforms(&[]),
            ["native", "gotify"],
            "all enabled"
        );
        assert_eq!(
            c.resolve_platforms(&["gotify".into(), "telegram".into()]),
            ["gotify"]
        );
        c.native.enabled = false;
        c.gotify.enabled = false;
        assert!(
            c.resolve_platforms(&[]).is_empty(),
            "defaults are filtered too"
        );
    }

    #[test]
    fn schema_shape() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "notify");
        assert_eq!(v["sections"].as_array().map(Vec::len), Some(4));
        let general = &v["sections"][0]["fields"];
        assert_eq!(general[0]["key"], "defaultPlatforms");
        assert_eq!(general[0]["type"], "multiselect");
        assert_eq!(general[0]["emptyLabel"], "none");
        let events_page = &v["sections"][2]["fields"][0];
        assert_eq!(events_page["type"], "page");
        assert_eq!(events_page["sections"].as_array().map(Vec::len), Some(8));
        assert_eq!(
            events_page["sections"][2]["fields"][0]["description"],
            "Send a notification when a run finishes (once per run, after every follow-up and continuation, not per message)."
        );
        assert_eq!(
            v["sections"][2]["fields"][1]["command"],
            "unipi:notify-test"
        );
        let platforms = &v["sections"][3]["fields"];
        assert_eq!(platforms[0]["sections"][0]["fields"][2]["type"], "secret");
        assert_eq!(platforms[2]["sections"][0]["fields"][4]["max"], 5.0);
    }
}
