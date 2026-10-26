//! `unipi.command-enchantment` — unipi `packages/autocomplete` (unipi-port §3.3).
//!
//! | unipi | Here |
//! |---|---|
//! | `addAutocompleteProvider` wrapping pi's base provider | guest `complete-input` returning one group that replaces the frontend's slash group ([`provider`]) |
//! | `COMMAND_REGISTRY`, descriptions, labels, `PACKAGE_COLORS` | [`registry`] (colours → OUS categorical tones) |
//! | 4-tier cross-group sort | [`sorting`] |
//! | pi-tui `fuzzyMatch`/`fuzzyFilter` | [`fuzzy`] |
//! | setting `command-enchantment.autocompleteEnhanced` | [`settings`]; off ⇒ no contribution (pure pass-through) |
//!
//! Argument completions (`/unipi:plan <Tab>`) stay with the host's `complete-command`; unipi's
//! `force: false` re-trigger fix is a frontend concern (see deviations).

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod fuzzy;
pub mod provider;
pub mod registry;
pub mod settings;
pub mod sorting;

use oino_extension::prelude::*;
use unipi_kit::settings::Store;
use unipi_kit::topics::ModuleReady;

use crate::provider::{CommandInfo, line_before_cursor, suggest};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.command-enchantment";

/// The extension.
#[derive(Default)]
pub struct CommandEnchantment {
    settings: Store,
}

impl CommandEnchantment {
    fn enabled(&self) -> bool {
        self.settings
            .get(settings::NAMESPACE)
            .bool("autocompleteEnhanced", true)
    }

    /// The session's commands: pi built-ins plus `control.commands-list`.
    fn commands() -> Vec<CommandInfo> {
        let mut out = CommandInfo::builtins();
        if let Ok(list) = control::commands() {
            out.extend(list.iter().filter_map(CommandInfo::from_json));
        }
        out
    }

    fn complete(&self, request: &Value) -> Option<Value> {
        if !self.enabled() {
            return None;
        }
        let text = request.get("text")?.as_str()?;
        let cursor = request
            .get("cursor")
            .and_then(Value::as_u64)
            .and_then(|c| usize::try_from(c).ok());
        let line = line_before_cursor(text, cursor);
        let group = suggest(&line, &Self::commands())?;
        serde_json::to_value(vec![group]).ok()
    }
}

impl Extension for CommandEnchantment {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings
            .declare(settings::NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.on(EventKind::SessionStart, |ext, _ev, _cx| async move {
            ext.settings.refresh();
            unipi_kit::bus::emit(&ModuleReady {
                name: unipi_kit::suite::pkg::COMMAND_ENCHANTMENT.npm.to_owned(),
                version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
                commands: Vec::new(),
                tools: Vec::new(),
                load_time_ms: None,
            });
            Ok(())
        });
        cx.input_completer(|ext, request, _cx| async move { ext.complete(&request) });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(CommandEnchantment);
