//! [`Extension`] trait and the [`Init`] registration collector.

use std::rc::Rc;
use std::time::Duration;

use futures::future::LocalBoxFuture;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::registry as wr;
use crate::command::{Command, Flag, Shortcut};
use crate::context::Ctx;
use crate::events::{Event, EventKind, IntoEventResult};
use crate::rt::Handlers;
use crate::tool::Tool;

/// An Oino extension. One value per component instance (created with `Default`, then leaked
/// so handlers receive `&'static Self`).
pub trait Extension: Default + 'static {
    /// Registers everything; runs once per instance (again after a trap restart).
    fn init(&'static self, cx: &mut Init<Self>) -> impl Future<Output = Result<()>>;

    /// Provider exports for components built with feature `provider`.
    #[cfg(feature = "provider")]
    fn provider(&'static self) -> Option<&'static dyn crate::provider::Provider> {
        None
    }
}

/// Registration collector passed to [`Extension::init`].
pub struct Init<E: 'static> {
    pub(crate) ext: &'static E,
    pub(crate) ctx: Ctx,
    pub(crate) manifest: Value,
    pub(crate) settings: Value,
    pub(crate) flags: Value,
    pub(crate) registration: wr::Registration,
    pub(crate) handlers: Handlers,
    pub(crate) timers: Vec<(String, Duration, bool)>,
}

impl<E: 'static> Init<E> {
    /// Init call context.
    #[must_use]
    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// Installed manifest.
    #[must_use]
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }

    /// Resolved settings (`{namespace: {...}}`).
    #[must_use]
    pub fn settings(&self) -> &Value {
        &self.settings
    }

    /// CLI flag values.
    #[must_use]
    pub fn flags(&self) -> &Value {
        &self.flags
    }

    /// Subscribes `f` to `kind` (pi `on`). Handlers of one extension run in registration order;
    /// the last non-`None` result wins.
    pub fn on<F, Fut, R>(&mut self, kind: EventKind, f: F) -> &mut Self
    where
        F: Fn(&'static E, Event, Ctx) -> Fut + 'static,
        Fut: Future<Output = Result<R>> + 'static,
        R: IntoEventResult,
    {
        if !self.registration.events.contains(&kind) {
            self.registration.events.push(kind);
        }
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.events.push((
            kind,
            Rc::new(move |ev, cx| {
                let f = Rc::clone(&f);
                Box::pin(
                    async move { f(ext, ev, cx).await.map(IntoEventResult::into_event_result) },
                ) as LocalBoxFuture<'static, _>
            }),
        ));
        self
    }

    /// Registers a tool.
    pub fn tool(&mut self, tool: Tool<E>) -> &mut Self {
        self.registration.tools.push(tool.spec.clone());
        self.handlers.install_tool(self.ext, tool);
        self
    }

    /// Registers a command.
    pub fn command(&mut self, command: Command<E>) -> &mut Self {
        self.registration.commands.push(command.spec.clone());
        self.handlers.install_command(self.ext, command);
        self
    }

    /// Registers a shortcut.
    pub fn shortcut(&mut self, shortcut: Shortcut<E>) -> &mut Self {
        self.registration.shortcuts.push(shortcut.spec.clone());
        self.handlers.install_shortcut(self.ext, shortcut);
        self
    }

    /// Registers a CLI flag.
    pub fn flag(&mut self, flag: Flag) -> &mut Self {
        self.registration.flags.push(flag.0);
        self
    }

    /// Registers a renderer for a custom message/entry type or tool name.
    pub fn renderer<F>(
        &mut self,
        target: wr::RendererTarget,
        key: impl Into<String>,
        f: F,
    ) -> &mut Self
    where
        F: Fn(&'static E, Value, Ctx) -> Option<Value> + 'static,
    {
        let key = key.into();
        self.registration.renderers.push(wr::RendererSpec {
            target,
            key: key.clone(),
        });
        let ext = self.ext;
        self.handlers
            .renderers
            .push((key, Rc::new(move |req, cx| f(ext, req, cx))));
        self
    }

    /// Serves `name` to other extensions (`services.call`).
    pub fn service<F, Fut>(&mut self, name: impl Into<String>, f: F) -> &mut Self
    where
        F: Fn(&'static E, Value, String, Ctx) -> Fut + 'static,
        Fut: Future<Output = Result<Value>> + 'static,
    {
        let name = name.into();
        self.registration.services.push(name.clone());
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.services.insert(
            name,
            Rc::new(move |payload, caller, cx| {
                let f = Rc::clone(&f);
                Box::pin(async move { f(ext, payload, caller, cx).await })
            }),
        );
        self
    }

    /// Settings namespace (`SettingsNamespace` JSON, ui-protocol §6.6).
    pub fn settings_namespace(&mut self, namespace: &Value) -> &mut Self {
        self.registration.settings.push(namespace.to_string());
        self
    }

    /// Arbiter nudge provider (runtime §7.6).
    pub fn nudge_provider<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&'static E, Ctx) -> Fut + 'static,
        Fut: Future<Output = Option<Value>> + 'static,
    {
        self.registration.arbiter.nudge_provider = true;
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.nudge = Some(Rc::new(move |cx| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, cx).await })
        }));
        self
    }

    /// Called when a proposed nudge was delivered.
    pub fn on_nudge_delivered(
        &mut self,
        f: impl Fn(&'static E, String, Ctx) + 'static,
    ) -> &mut Self {
        let ext = self.ext;
        self.handlers.nudge_delivered = Some(Rc::new(move |id, cx| f(ext, id, cx)));
        self
    }

    /// Arbiter wait source: `Some(reason)` keeps the agent from settling.
    pub fn wait_source<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&'static E, Ctx) -> Fut + 'static,
        Fut: Future<Output = Option<String>> + 'static,
    {
        self.registration.arbiter.wait_source = true;
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.wait_reason = Some(Rc::new(move |cx| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, cx).await })
        }));
        self
    }

    /// Virtual-model router (`route-model`).
    pub fn router<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&'static E, Value, Ctx) -> Fut + 'static,
        Fut: Future<Output = Result<Value>> + 'static,
    {
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.route = Some(Rc::new(move |req, cx| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, req, cx).await })
        }));
        self
    }

    /// Autocomplete contributor (`complete-input`).
    pub fn input_completer<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&'static E, Value, Ctx) -> Fut + 'static,
        Fut: Future<Output = Option<Value>> + 'static,
    {
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.complete_input = Some(Rc::new(move |req, cx| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, req, cx).await })
        }));
        self
    }

    /// Runs on host shutdown.
    pub fn on_shutdown<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&'static E, String) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let ext = self.ext;
        let f = Rc::new(f);
        self.handlers.shutdown = Some(Rc::new(move |reason| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, reason).await })
        }));
        self
    }

    /// Starts a timer delivering `oino_timer {timerId: id}` (subscribes to `OinoTimer`).
    pub fn timer(&mut self, id: impl Into<String>, delay: Duration, repeat: bool) -> &mut Self {
        if !self.registration.events.contains(&EventKind::OinoTimer) {
            self.registration.events.push(EventKind::OinoTimer);
        }
        self.timers.push((id.into(), delay, repeat));
        self
    }
}
