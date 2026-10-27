//! `unipi.utility` — unipi `packages/utility` (unipi-port §3.2), the grab-bag module.
//!
//! | unipi | Here |
//! |---|---|
//! | `/unipi:settings [search]` | merged into the core `/settings` hub (core commands win) |
//! | `/continue`, `/retry` | hidden `unipi-continue` message with `triggerTurn` ([`commands`]) |
//! | `/cleanup`, `/doctor` | [`maintenance`] (allowlisted own folders; host-based checks) |
//! | `/summarize [q]` + bundled `summarize` skill | `/skill:summarize` user message with provenance ([`commands`]) |
//! | auto-rename (jev gate + one-tool in-memory session) + `rename-now` | [`rename`]: nested agent (fallback: one `models.complete`) + `session.set-session-name` |
//! | `/answer [reply\|questions\|web]` + hint widget | [`answer`]: custom/editor dialogs, loopback `http.listener` form |
//! | attachments (`onTerminalInput`, paste scan) | [`attach`]: `oino_editor` intents + `input` transform + entry renderer |
//! | `image_generate`, `image_edit`, `image_recognize` + recognize gating + image model cache | [`image`] |
//! | `/move` + move handlers | [`relink`]: sessions, git worktrees, `move.handle` fan-out |
//! | render styles, harness panels, delegated panel | [`render`]: tool/message renderers, `utility.render-delegated` |
//! | settings `utility`, `image` | [`settings`] |
//! | `/cron`, browser state, Herdr sync | not ported (unipi-port §3.2, D22) |

//!
//! # Errors
//!
//! Fallible functions fail with the host error of the import they call (permission denied,
//! unsupported capability, stale context, …) or with the message shown to the user or the
//! agent; individual functions do not repeat an `# Errors` section.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "errors are documented once at crate level"
)]

pub mod answer;
pub mod attach;
pub mod chatter;
pub mod commands;
pub mod hostfs;
pub mod image;
pub mod maintenance;
pub mod relink;
pub mod rename;
pub mod render;
pub mod settings;

use std::io::Write as _;

use oino_extension::prelude::*;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::settings::Store;
use unipi_kit::topics::ModuleReady;

use crate::settings::{IMAGE_NAMESPACE, ImageConfig, NAMESPACE, UtilSettings};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.utility";

/// Commands listed in `MODULE_READY` (unipi `ALL_COMMANDS`).
pub const COMMANDS: [&str; 6] = ["continue", "retry", "cleanup", "doctor", "answer", "move"];

/// The extension.
#[derive(Default)]
pub struct Utility {
    settings: Store,
    renamer: rename::Renamer,
    answer: answer::Answerer,
    attach: attach::Attachments,
}

/// Runs a detached task (the round never waits on naming).
pub fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

/// Decision Model settings for the `utility` consumer.
#[must_use]
pub fn jev_settings() -> unipi_kit::jev::Settings {
    let all = control::settings()
        .ok()
        .and_then(|s| s.get("extensionSettings").cloned())
        .unwrap_or(Value::Null);
    unipi_kit::jev::Settings::resolve(&all, Some(NAMESPACE))
}

impl Utility {
    /// Current `utility` settings.
    #[must_use]
    pub fn util(&self) -> UtilSettings {
        UtilSettings::from_json(&self.settings.get(NAMESPACE).0)
    }

    /// Current `image` settings (secret fields filled from the secret store).
    #[must_use]
    pub fn image(&self) -> ImageConfig {
        image::with_secrets(ImageConfig::from_json(
            &self.settings.get(IMAGE_NAMESPACE).0,
        ))
    }

    /// Debug trail line (`/logs/<file>`), only with `utility.debug`.
    pub fn debug_log(&self, file: &str, line: &str) {
        if !self.util().debug {
            return;
        }
        let _ = std::fs::create_dir_all("/logs");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("/logs/{file}"))
        {
            let _ = writeln!(
                f,
                "{} {line}",
                unipi_kit::fmt::iso_from_ms(timers::now_ms())
            );
        }
    }

    fn render(&self, req: &Value, cx: &Ctx) -> Option<Value> {
        let util = self.util();
        let request = render::Request::parse(req);
        render::render(
            &request,
            util.render,
            &cx.cwd().to_string_lossy(),
            hostfs::home().as_deref(),
            util.attachments.preview,
        )
    }

    fn on_session_start(&self, cx: &Ctx) {
        self.settings.refresh();
        let sid = cx.session_id();
        self.renamer.on_session_start(sid);
        if cx.is_child() {
            return;
        }
        let mut tools = Vec::new();
        let c = self.image();
        for (on, name) in [
            (c.generate.enabled, image::GENERATE),
            (c.edit.enabled, image::EDIT),
            (c.recognize.enabled, image::RECOGNIZE),
        ] {
            if on {
                tools.push(name.to_owned());
            }
        }
        unipi_kit::bus::emit(&ModuleReady {
            name: unipi_kit::suite::pkg::UTILITY.npm.to_owned(),
            version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
            commands: COMMANDS.iter().map(|c| (*c).to_owned()).collect(),
            tools,
            load_time_ms: None,
        });
        self.attach.reset(sid);
        if c.recognize.enabled {
            image::gate(None);
        }
        spawn(async {
            let _ = image::refresh_cache().await;
        });
    }

    fn on_input(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        if cx.is_child() {
            return None;
        }
        let sid = cx.session_id();
        self.renamer.on_input(sid, ev);
        self.answer.clear_hint(sid);
        let util = self.util();
        if util.attachments.enabled {
            return self.attach.on_input(sid, ev, util.attachments.preview);
        }
        None
    }
}

fn register_commands(cx: &mut Init<Utility>) {
    cx.command(
        Command::<Utility>::new("continue")
            .description("Continue the agent from where it left off, without adding text (/retry)")
            .run(|_ext, _args, cx| async move { commands::continue_turn(&cx).await }),
    );
    cx.command(
        Command::<Utility>::new("retry")
            .description("Retry the last turn — alias of /continue")
            .run(|_ext, _args, cx| async move { commands::continue_turn(&cx).await }),
    );
    cx.command(
        Command::<Utility>::new("cleanup")
            .description("Remove stale Oino temp files and leftovers (preview first)")
            .argument_hint("[--dry-run] [--yes]")
            .run(|_ext, args, cx| async move { commands::cleanup(&args, &cx).await }),
    );
    cx.command(
        Command::<Utility>::new("doctor")
            .description(
                "Check Oino's runtime: folders, config, model registry, Decision Model, skills",
            )
            .run(|_ext, _args, cx| async move { commands::doctor(&cx).await }),
    );
    cx.command(
        Command::<Utility>::new(answer::COMMAND)
            .description("Answer the last reply — a fixed input box under the scrollable reply (reply), per question (questions), or a browser form (web)")
            .argument_hint("[reply|questions|web]")
            .run(|ext, args, cx| async move { ext.answer.run(&args, &cx, ext.util().answer).await })
            .complete(|_ext, prefix, _cx| async move { answer::complete(&prefix) }),
    );
    cx.command(
        Command::<Utility>::new("summarize")
            .description("Summarize this session for you (answer first, then findings, open items, questions); add text to set a focus")
            .argument_hint("[focus]")
            .run(|_ext, args, cx| async move { commands::summarize(&args, &cx).await }),
    );
    cx.command(
        Command::<Utility>::new(relink::COMMAND)
            .description("Re-link state after moving this project: [<old-path>] [--dry-run] (never moves the project itself)")
            .argument_hint("[<old-path>] [--dry-run]")
            .run(|_ext, args, cx| async move { relink::run(&args, &cx).await }),
    );
    cx.command(
        Command::<Utility>::new(rename::RUNNER)
            .description("Name the session from the recent requests right away")
            .hidden()
            .run(
                |ext, _args, cx| async move { ext.renamer.rename_now(ext, cx.session_id()).await },
            ),
    );
}

fn tool(
    name: &str,
    label: &str,
    description: &str,
    snippet: &str,
    guidelines: &[&str],
    parameters: &Value,
) -> Tool<Utility> {
    let mut t = Tool::<Utility>::new(name)
        .label(label)
        .description(description)
        .prompt_snippet(snippet)
        .parameters(parameters);
    for g in guidelines {
        t = t.guideline(*g);
    }
    t
}

fn register_tools(cx: &mut Init<Utility>, config: &ImageConfig) {
    for (name, label, description, snippet, guidelines, parameters) in image::tool_defs() {
        let t =
            tool(name, label, description, snippet, &guidelines, &parameters).kind(ToolKind::Other);
        match name {
            image::GENERATE if config.generate.enabled => {
                cx.tool(t.run(|ext, a: image::GenerateArgs, _cx| async move {
                    image::run_generate(&ext.image(), false, &a.prompt, a.model.as_deref(), None)
                        .await
                }));
            }
            image::EDIT if config.edit.enabled => {
                cx.tool(t.run(|ext, a: image::EditArgs, cx| async move {
                    let src = image::load(&a.image, &cx.cx.cwd().to_string_lossy())?;
                    let payload = image::transports::ImagePayload {
                        data: src.data,
                        mime_type: src.mime_type,
                    };
                    image::run_generate(
                        &ext.image(),
                        true,
                        &a.prompt,
                        a.model.as_deref(),
                        Some(payload),
                    )
                    .await
                }));
            }
            image::RECOGNIZE if config.recognize.enabled => {
                cx.tool(t.kind(ToolKind::Read).run(
                    |ext, a: image::RecognizeArgs, cx| async move {
                        image::run_recognize(&ext.image(), &a, &cx.cx.cwd().to_string_lossy()).await
                    },
                ));
            }
            _ => {}
        }
    }
    cx.tool(
        Tool::<Utility>::new(rename::session::TOOL)
            .label("Rename session")
            .description("Set the session title (2-6 words, Title Case).")
            .parameters(&rename::session::tool_parameters())
            .inactive()
            .run(|ext, a: Value, cx| async move {
                if cx.cx.child_role() != Some(rename::session::CHILD_ROLE) {
                    return Ok(ToolResult::error(
                        "rename_session is only available inside the session-naming side session.",
                    ));
                }
                let name = rename::gate::sanitize_name(
                    a.get("name").and_then(Value::as_str).unwrap_or_default(),
                );
                if name.is_empty() {
                    return Ok(ToolResult::text("Error: empty title."));
                }
                ext.renamer.record_choice(&name);
                Ok(ToolResult::text(format!("Title set: {name}"))
                    .with_details(json!({"name": name})))
            }),
    );
}

fn register_renderers(cx: &mut Init<Utility>) {
    for t in render::tools::BUILTINS {
        cx.renderer(RendererTarget::Tool, t, |ext, req, cx| {
            ext.render(&req, &cx)
        });
    }
    for (custom_type, ..) in render::harness::KNOWN {
        cx.renderer(
            RendererTarget::CustomMessage,
            custom_type,
            |ext, req, cx| ext.render(&req, &cx),
        );
    }
    cx.renderer(
        RendererTarget::CustomEntry,
        attach::ENTRY,
        |ext, req, cx| ext.render(&req, &cx),
    );
}

fn register_services(cx: &mut Init<Utility>) {
    cx.service(image::MODELS_SERVICE, |ext, _p, _caller, _cx| async move {
        Ok(image::model_options(&ext.image()))
    });
    cx.service(render::STYLE_SERVICE, |ext, _p, _caller, _cx| async move {
        Ok(json!({"style": ext.util().render.as_str()}))
    });
    cx.service(
        render::DELEGATED_SERVICE,
        |ext, p, _caller, _cx| async move {
            let steps = p
                .get("steps")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let style = p
                .get("style")
                .and_then(Value::as_str)
                .and_then(settings::RenderStyle::parse)
                .unwrap_or_else(|| ext.util().render);
            let expanded = p.get("expanded").and_then(Value::as_bool).unwrap_or(false);
            Ok(render::delegated::render(
                &steps,
                p.get("label").and_then(Value::as_str),
                style,
                expanded,
            ))
        },
    );
}

fn register_events(cx: &mut Init<Utility>) {
    cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
        ext.on_session_start(&cx);
        Ok(())
    });
    cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
        ext.renamer.on_session_shutdown(cx.session_id());
        Ok(())
    });
    cx.on(EventKind::Input, |ext, ev, cx| async move {
        Ok(ext.on_input(&ev, &cx))
    });
    cx.on(EventKind::BeforeAgentStart, |ext, _ev, cx| async move {
        ext.answer.clear_hint(cx.session_id());
        Ok(())
    });
    cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
        ext.attach.on_agent_start(cx.session_id());
        Ok(())
    });
    cx.on(EventKind::AgentEnd, |ext, ev, cx| async move {
        if cx.is_child() {
            return Ok(());
        }
        ext.renamer.on_agent_end(ext, cx.session_id(), &ev);
        ext.answer.on_agent_end(&cx, ext.util().answer.hint);
        Ok(())
    });
    cx.on(EventKind::ModelSelect, |ext, ev, cx| async move {
        if !cx.is_child() && ext.image().recognize.enabled {
            image::gate(ev.get("model"));
        }
        Ok(())
    });
    cx.on(EventKind::OinoEditor, |ext, ev, cx| async move {
        let util = ext.util();
        if !util.attachments.enabled {
            return Ok(None);
        }
        Ok(ext
            .attach
            .on_editor(cx.session_id(), &ev, util.attachments.preview))
    });
    cx.on(EventKind::OinoUiAction, |ext, ev, _cx| async move {
        ext.answer.on_ui_action(&ev);
        Ok(())
    });
    cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
        let _ = ext.settings.on_changed(&ev.payload);
        Ok(())
    });
}

impl Extension for Utility {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let util_ns = settings::namespace(EXTENSION_ID);
        let image_ns = settings::image_namespace(EXTENSION_ID);
        self.settings.declare(NAMESPACE, util_ns.defaults.clone());
        self.settings
            .declare(IMAGE_NAMESPACE, image_ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&util_ns.to_json());
        cx.settings_namespace(&image_ns.to_json());
        register_commands(cx);
        register_tools(
            cx,
            &ImageConfig::from_json(&self.settings.get(IMAGE_NAMESPACE).0),
        );
        register_renderers(cx);
        register_services(cx);
        register_events(cx);
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Utility);
