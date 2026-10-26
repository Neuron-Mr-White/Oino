//! Command, shortcut and flag builders (pi `registerCommand`, `registerShortcut`, `registerFlag`).

use std::rc::Rc;

use futures::future::LocalBoxFuture;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::registry as wr;
use crate::context::{CommandCtx, Ctx};

/// A completion item for command arguments.
pub use crate::bindings::exports::oino::extension::guest::CompletionItem;

pub(crate) type CommandRun<E> =
    Rc<dyn Fn(&'static E, String, CommandCtx) -> LocalBoxFuture<'static, Result<Option<Value>>>>;
pub(crate) type CommandComplete<E> =
    Rc<dyn Fn(&'static E, String, Ctx) -> LocalBoxFuture<'static, Vec<CompletionItem>>>;

/// A slash command bound to extension type `E`.
pub struct Command<E: 'static> {
    pub(crate) spec: wr::CommandSpec,
    pub(crate) run: Option<CommandRun<E>>,
    pub(crate) complete: Option<CommandComplete<E>>,
}

impl<E: 'static> Command<E> {
    /// Command `name` (without `/`).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            spec: wr::CommandSpec {
                name: name.into(),
                description: String::new(),
                argument_hint: None,
                completions: false,
                hidden: false,
            },
            run: None,
            complete: None,
        }
    }

    /// Description.
    #[must_use]
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.spec.description = d.into();
        self
    }

    /// Argument hint.
    #[must_use]
    pub fn argument_hint(mut self, h: impl Into<String>) -> Self {
        self.spec.argument_hint = Some(h.into());
        self
    }

    /// Invokable but not listed.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.spec.hidden = true;
        self
    }

    /// Handler: `(ext, args, cx) -> Result<()>`.
    #[must_use]
    pub fn run<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(&'static E, String, CommandCtx) -> Fut + 'static,
        Fut: Future<Output = Result<()>> + 'static,
    {
        let f = Rc::new(f);
        self.run = Some(Rc::new(move |ext, args, cx| {
            let f = Rc::clone(&f);
            Box::pin(async move { f(ext, args, cx).await.map(|()| None) })
        }));
        self
    }

    /// Handler returning a JSON value (captured by `control.command-invoke` callers).
    #[must_use]
    pub fn run_value<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(&'static E, String, CommandCtx) -> Fut + 'static,
        Fut: Future<Output = Result<Option<Value>>> + 'static,
    {
        let f = Rc::new(f);
        self.run = Some(Rc::new(move |ext, args, cx| Box::pin(f(ext, args, cx))));
        self
    }

    /// Argument completions (pi `getArgumentCompletions`).
    #[must_use]
    pub fn complete<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(&'static E, String, Ctx) -> Fut + 'static,
        Fut: Future<Output = Vec<CompletionItem>> + 'static,
    {
        self.spec.completions = true;
        let f = Rc::new(f);
        self.complete = Some(Rc::new(move |ext, prefix, cx| Box::pin(f(ext, prefix, cx))));
        self
    }
}

/// A completion item with a value only.
#[must_use]
pub fn item(value: impl Into<String>) -> CompletionItem {
    CompletionItem {
        value: value.into(),
        label: None,
        description: None,
    }
}

pub(crate) type ShortcutRun<E> =
    Rc<dyn Fn(&'static E, CommandCtx) -> LocalBoxFuture<'static, Result<()>>>;

/// A keyboard shortcut bound to extension type `E`.
pub struct Shortcut<E: 'static> {
    pub(crate) spec: wr::ShortcutSpec,
    pub(crate) run: Option<ShortcutRun<E>>,
}

impl<E: 'static> Shortcut<E> {
    /// Shortcut action `id` with default keys (e.g. `"ctrl+shift+b"`).
    pub fn new(id: impl Into<String>, keys: &[&str]) -> Self {
        Self {
            spec: wr::ShortcutSpec {
                id: id.into(),
                default_keys: keys.iter().map(ToString::to_string).collect(),
                description: String::new(),
            },
            run: None,
        }
    }

    /// Description.
    #[must_use]
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.spec.description = d.into();
        self
    }

    /// Handler (runs with a command context).
    #[must_use]
    pub fn run<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(&'static E, CommandCtx) -> Fut + 'static,
        Fut: Future<Output = Result<()>> + 'static,
    {
        let f = Rc::new(f);
        self.run = Some(Rc::new(move |ext, cx| Box::pin(f(ext, cx))));
        self
    }
}

/// A CLI flag (`--name`).
pub struct Flag(pub(crate) wr::FlagSpec);

impl Flag {
    /// Boolean flag.
    pub fn boolean(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self(wr::FlagSpec {
            name: name.into(),
            type_: wr::FlagType::Boolean,
            default_value: None,
            description: description.into(),
        })
    }

    /// String flag.
    pub fn string(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self(wr::FlagSpec {
            name: name.into(),
            type_: wr::FlagType::String,
            default_value: None,
            description: description.into(),
        })
    }

    /// Default value.
    #[must_use]
    pub fn default_value(mut self, v: &Value) -> Self {
        self.0.default_value = Some(v.to_string());
        self
    }
}

/// Current value of a flag (pi `getFlag`).
#[must_use]
pub fn get_flag(name: &str) -> Option<Value> {
    crate::bindings::oino::extension::registry::get_flag(name)
        .and_then(|t| serde_json::from_str(&t).ok())
}
