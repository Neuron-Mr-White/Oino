//! Tool builder and results (pi `registerTool`).

use std::rc::Rc;

use futures::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bindings::oino::extension::registry as wr;
pub use crate::bindings::oino::extension::registry::ToolKind;
use crate::context::ToolCtx;
use crate::{Error, Result};

/// `AgentToolResult` (extension-api §4.2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// Content blocks (`{type:"text", text}` / `{type:"image", data, mimeType}`).
    pub content: Vec<Value>,
    /// Details for renderers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Structured content (output schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// Error result.
    #[serde(default)]
    pub is_error: bool,
    /// End the agent run after this result.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminate: bool,
}

impl ToolResult {
    /// A text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![json!({"type": "text", "text": text.into()})],
            ..Self::default()
        }
    }

    /// An error result.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(text)
        }
    }

    /// An image result (base64 data).
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            content: vec![
                json!({"type": "image", "data": data.into(), "mimeType": mime_type.into()}),
            ],
            ..Self::default()
        }
    }

    /// Adds details.
    #[must_use]
    pub fn with_details(mut self, details: impl Serialize) -> Self {
        self.details = serde_json::to_value(details).ok();
        self
    }

    /// Ends the run after this result.
    #[must_use]
    pub fn terminate(mut self) -> Self {
        self.terminate = true;
        self
    }
}

pub(crate) type ToolRun = Rc<dyn Fn(Value, ToolCtx) -> LocalBoxFuture<'static, Result<ToolResult>>>;
/// Tool implementation still bound to its extension value.
pub(crate) type BoundRun<E> =
    Rc<dyn Fn(&'static E, Value, ToolCtx) -> LocalBoxFuture<'static, Result<ToolResult>>>;
pub(crate) type JsonMap = Rc<dyn Fn(Value) -> Value>;
pub(crate) type JsonMaybe = Rc<dyn Fn(Value) -> Option<Value>>;

/// JSON schema of `A` (draft 2020-12, without `$schema`).
#[must_use]
pub fn schema_of<A: JsonSchema>() -> Value {
    let mut value = serde_json::to_value(schemars::schema_for!(A))
        .unwrap_or_else(|_| json!({"type": "object"}));
    if let Some(obj) = value.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    value
}

/// A tool definition bound to extension type `E`.
pub struct Tool<E: 'static> {
    pub(crate) spec: wr::ToolSpec,
    pub(crate) run: Option<BoundRun<E>>,
    pub(crate) prepare_arguments: Option<JsonMap>,
    pub(crate) prepare_loadout: Option<JsonMaybe>,
    pub(crate) reaches: Option<JsonMap>,
}

impl<E: 'static> Tool<E> {
    /// A tool named `name` (label defaults to the name, parameters to `{}` object).
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            spec: wr::ToolSpec {
                label: name.clone(),
                name,
                description: String::new(),
                parameters: json!({"type": "object", "properties": {}}).to_string(),
                output_schema: None,
                prompt_snippet: None,
                prompt_guidelines: Vec::new(),
                exposure: wr::Exposure::Direct,
                default_active: true,
                execution_mode: wr::ExecutionMode::Parallel,
                replay: None,
                kind: wr::ToolKind::Other,
                namespace: None,
                annotations: None,
                reaches_tools: wr::ReachesTools::None,
                prepare_arguments: false,
                prepare_loadout: false,
                render: None,
            },
            run: None,
            prepare_arguments: None,
            prepare_loadout: None,
            reaches: None,
        }
    }

    /// Tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.spec.name
    }

    /// Frontend label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.spec.label = label.into();
        self
    }

    /// Model-facing description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.spec.description = description.into();
        self
    }

    /// Parameters from a `JsonSchema` type.
    #[must_use]
    pub fn args<A: JsonSchema>(mut self) -> Self {
        self.spec.parameters = schema_of::<A>().to_string();
        self
    }

    /// Raw parameters schema.
    #[must_use]
    pub fn parameters(mut self, schema: &Value) -> Self {
        self.spec.parameters = schema.to_string();
        self
    }

    /// Output schema.
    #[must_use]
    pub fn output_schema(mut self, schema: &Value) -> Self {
        self.spec.output_schema = Some(schema.to_string());
        self
    }

    /// pi `promptSnippet`.
    #[must_use]
    pub fn prompt_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.spec.prompt_snippet = Some(snippet.into());
        self
    }

    /// pi `promptGuidelines`.
    #[must_use]
    pub fn guideline(mut self, line: impl Into<String>) -> Self {
        self.spec.prompt_guidelines.push(line.into());
        self
    }

    /// Exposure.
    #[must_use]
    pub fn exposure(mut self, exposure: wr::Exposure) -> Self {
        self.spec.exposure = exposure;
        self
    }

    /// Inactive until activated (`defaultActive: false`).
    #[must_use]
    pub fn inactive(mut self) -> Self {
        self.spec.default_active = false;
        self
    }

    /// Forces sequential batches.
    #[must_use]
    pub fn sequential(mut self) -> Self {
        self.spec.execution_mode = wr::ExecutionMode::Sequential;
        self
    }

    /// ACP tool kind.
    #[must_use]
    pub fn kind(mut self, kind: ToolKind) -> Self {
        self.spec.kind = kind;
        self
    }

    /// Replay policy.
    #[must_use]
    pub fn replay(mut self, replay: wr::Replay) -> Self {
        self.spec.replay = Some(replay);
        self
    }

    /// Namespace.
    #[must_use]
    pub fn namespace(mut self, ns: wr::ToolNamespace) -> Self {
        self.spec.namespace = Some(ns);
        self
    }

    /// Annotations.
    #[must_use]
    pub fn annotations(mut self, a: wr::ToolAnnotations) -> Self {
        self.spec.annotations = Some(a);
        self
    }

    /// Tools this tool may call through `control::execute_tool`.
    #[must_use]
    pub fn reaches(mut self, reaches: wr::ReachesTools) -> Self {
        self.spec.reaches_tools = reaches;
        self
    }

    /// Dynamic reach decided per input (`"all"` or `["name", …]`).
    #[must_use]
    pub fn reaches_dynamic(mut self, f: impl Fn(Value) -> Value + 'static) -> Self {
        self.spec.reaches_tools = wr::ReachesTools::Dynamic;
        self.reaches = Some(Rc::new(f));
        self
    }

    /// pi `prepareArguments`.
    #[must_use]
    pub fn prepare_arguments(mut self, f: impl Fn(Value) -> Value + 'static) -> Self {
        self.spec.prepare_arguments = true;
        self.prepare_arguments = Some(Rc::new(f));
        self
    }

    /// pi `prepareLoadout`.
    #[must_use]
    pub fn prepare_loadout(mut self, f: impl Fn(Value) -> Option<Value> + 'static) -> Self {
        self.spec.prepare_loadout = true;
        self.prepare_loadout = Some(Rc::new(f));
        self
    }

    /// `ToolRenderHints` JSON (ui-protocol §6.4).
    #[must_use]
    pub fn render_hints(mut self, hints: &Value) -> Self {
        self.spec.render = Some(hints.to_string());
        self
    }

    /// The tool implementation: receives `&'static E`, decoded arguments and a [`ToolCtx`].
    #[must_use]
    pub fn run<A, F, Fut>(mut self, f: F) -> Self
    where
        A: DeserializeOwned + 'static,
        F: Fn(&'static E, A, ToolCtx) -> Fut + 'static,
        Fut: Future<Output = Result<ToolResult>> + 'static,
    {
        let f = Rc::new(f);
        self.run = Some(Rc::new(move |ext, args, cx| {
            let f = Rc::clone(&f);
            Box::pin(async move {
                let args: A = serde_json::from_value(args)
                    .map_err(|e| Error::msg(format!("invalid arguments: {e}")))?;
                f(ext, args, cx).await
            })
        }));
        self
    }

    /// Registers this tool dynamically (after `init`).
    ///
    /// # Errors
    ///
    /// The host rejected the registration.
    pub fn register(self, ext: &'static E) -> Result<()> {
        let spec = self.spec.clone();
        crate::rt::install_tool(ext, self);
        crate::bindings::oino::extension::registry::register_tool(&spec)?;
        Ok(())
    }
}
