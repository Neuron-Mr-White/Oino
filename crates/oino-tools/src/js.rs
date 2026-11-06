//! JavaScript semantics the tools rely on: number formatting, `toFixed`, UTF-16 lengths and
//! `String.prototype.trimEnd`.

/// JavaScript `Number.prototype.toString()` for the values tools print (counts, offsets,
/// timeouts).
#[must_use]
pub(crate) fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    if (1e-6..1e21).contains(&magnitude) {
        // Rust's shortest round-trip representation matches JavaScript in this range.
        return format!("{value}");
    }
    // Exponent form: `1e+21`, `1.5e-7`.
    let formatted = format!("{value:e}");
    match formatted.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => formatted,
    }
}

/// JavaScript `Number.prototype.toFixed(digits)` for finite non-negative values below `1e21`
/// (ties round up, on the exact binary value).
#[must_use]
pub(crate) fn to_fixed(value: f64, digits: usize) -> String {
    let negative = value < 0.0;
    let exact = format!("{:.60}", value.abs());
    let Some((int_part, frac_part)) = exact.split_once('.') else {
        return exact;
    };
    let mut kept: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(digits))
        .collect();
    let round_up = frac_part
        .as_bytes()
        .get(digits)
        .is_some_and(|digit| *digit >= b'5');
    if round_up {
        let mut index = kept.len();
        loop {
            if index == 0 {
                kept.insert(0, b'1');
                break;
            }
            index -= 1;
            if kept[index] == b'9' {
                kept[index] = b'0';
            } else {
                kept[index] += 1;
                break;
            }
        }
    }
    let int_len = kept.len() - digits;
    let mut out = String::with_capacity(kept.len() + 2);
    if negative && kept.iter().any(|digit| *digit != b'0') {
        out.push('-');
    }
    out.push_str(&String::from_utf8_lossy(&kept[..int_len]));
    if digits > 0 {
        out.push('.');
        out.push_str(&String::from_utf8_lossy(&kept[int_len..]));
    }
    out
}

/// A count as a JavaScript number.
#[must_use]
pub(crate) fn count_f64(value: usize) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "counts are far below 2^53")]
    let value = value as f64;
    value
}

/// JavaScript `string.length` (UTF-16 code units).
#[must_use]
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// JavaScript `string.slice(0, units)` measured in UTF-16 code units. A surrogate pair cut in
/// half leaves a lone surrogate in JavaScript, which becomes U+FFFD once encoded as UTF-8.
#[must_use]
pub(crate) fn utf16_prefix(text: &str, units: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.len_utf16();
        if used + width > units {
            if used < units {
                out.push(char::REPLACEMENT_CHARACTER);
            }
            break;
        }
        out.push(ch);
        used += width;
    }
    out
}

/// Whether JavaScript's `\s` / `trim` treat `ch` as whitespace (`WhiteSpace` + `LineTerminator`).
#[must_use]
pub(crate) fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript `String.prototype.trimEnd`.
#[must_use]
pub(crate) fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_js_whitespace)
}

/// JavaScript `String.prototype.trim`.
#[must_use]
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_javascript() {
        assert_eq!(number_to_string(3.0), "3");
        assert_eq!(number_to_string(0.5), "0.5");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(1.5e-7), "1.5e-7");
        assert_eq!(number_to_string(f64::INFINITY), "Infinity");
        assert_eq!(number_to_string(-2.0), "-2");
    }

    #[test]
    fn to_fixed_rounds_ties_up() {
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(1.05, 1), "1.1");
        assert_eq!(to_fixed(50.0, 1), "50.0");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(1.5, 2), "1.50");
        assert_eq!(to_fixed(0.0, 1), "0.0");
    }

    #[test]
    fn utf16_helpers() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(utf16_prefix("a😀b", 2), "a\u{fffd}");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(trim_end("x \u{feff}\u{a0}"), "x");
    }
}

// The sandbox deliberately has no module loader, filesystem, network or timers.
use base64::Engine as _;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{JsonObject, JsonValue, UserContent};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Function, Promise, async_with};
use serde_json::json;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;

/// Sandboxed JavaScript tool. All external effects use the session's nested dispatcher.
#[derive(Debug)]
pub struct CodemodeTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl CodemodeTool {
    /// Construct the sandbox tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: crate::info::codemode_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for CodemodeTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        Box::pin(async move { run_script(config, call, cancel).await })
    }
}

/// Model-initiated discovery and append-only loading of schemas.
#[derive(Debug)]
pub struct ToolSearchTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl ToolSearchTool {
    /// Construct the discovery tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: crate::info::tool_search_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for ToolSearchTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }
    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let context = Arc::clone(&self.config.context);
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(crate::aborted());
            }
            let query = call
                .args
                .get("query")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| ToolError::new("query must be a string"))?;
            let limit = call
                .args
                .get("max_results")
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|v| *v > 0)
                        .ok_or_else(|| {
                            ToolError::new("max_results must be a positive integer or null")
                        })
                })
                .transpose()?;
            let value = context.search_tools(query, limit)?;
            Ok(AgentToolResult {
                structured_content: Some(value.clone()),
                ..AgentToolResult::text(value.to_string())
            })
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptOptions {
    #[serde(default = "default_output_tokens")]
    max_output_tokens: usize,
    timeout_ms: Option<u64>,
}
fn default_output_tokens() -> usize {
    10_000
}

fn script_options(source: &str) -> Result<ScriptOptions, ToolError> {
    let mut options = ScriptOptions {
        max_output_tokens: default_output_tokens(),
        timeout_ms: None,
    };
    for (index, line) in source.lines().enumerate() {
        if let Some(value) = line.trim_start().strip_prefix("// @options:") {
            if index != 0 {
                return Err(ToolError::new("@options must be the first line"));
            }
            options = serde_json::from_str(value)
                .map_err(|e| ToolError::new(format!("Invalid @options: {e}")))?;
        }
    }
    Ok(options)
}

static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();

async fn run_script(
    config: ToolConfig,
    call: ToolInvocation,
    cancel: CancellationToken,
) -> Result<AgentToolResult, ToolError> {
    let source = call
        .args
        .get("code")
        .or_else(|| call.args.get("script"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ToolError::new("code must be JavaScript source"))?
        .to_owned();
    let options = script_options(&source)?;
    let started = Instant::now();
    let deadline = options
        .timeout_ms
        .map(|ms| {
            started
                .checked_add(Duration::from_millis(ms))
                .ok_or_else(|| ToolError::new("timeout_ms exceeds the supported deadline range"))
        })
        .transpose()?;
    let workers = WORKERS
        .get_or_init(|| {
            Arc::new(Semaphore::new(
                std::env::var("PI_CODEMODE_MAX_WORKERS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(2),
            ))
        })
        .clone();
    let wait = deadline.map_or(Duration::from_secs(60), |d| {
        d.saturating_duration_since(Instant::now())
            .min(Duration::from_secs(60))
    });
    let permit = tokio::select! {
        () = cancel.cancelled() => return Err(crate::aborted()),
        result = tokio::time::timeout(wait, workers.acquire_owned()) => result.map_err(|_| ToolError::new("Timed out waiting for a codemode worker"))?
            .map_err(|_| ToolError::new("Codemode worker pool closed"))?,
    };
    // A dedicated blocking worker makes QuickJS CPU loops interruptible without
    // starving the session's async executor. Keep its permit until the VM exits.
    let child = cancel.child_token();
    let worker_cancel = child.clone();
    let runtime = tokio::runtime::Handle::current();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.block_on(sandbox(
            &config,
            &call.tool_call_id,
            &source,
            worker_cancel,
            deadline,
            options.max_output_tokens,
            started,
        ))
    });
    let result = tokio::select! {
        () = cancel.cancelled() => { child.cancel(); Err(crate::aborted()) },
        result = task => result.map_err(|e| ToolError::new(format!("Codemode worker: {e}")))?,
    };
    child.cancel();
    result
}

type PendingRequest<'js> = (String, String, Function<'js>, Function<'js>);

fn make_dispatch<'js>(
    ctx: &rquickjs::Ctx<'js>,
    queue: std::rc::Rc<std::cell::RefCell<Vec<PendingRequest<'js>>>>,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: rquickjs::Ctx<'js>, name: String, args: String| {
            let (promise, resolve, reject) = ctx.promise()?;
            queue.borrow_mut().push((name, args, resolve, reject));
            Ok::<_, rquickjs::Error>(promise)
        },
    )
}

#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "one VM lifetime and ordered output transaction"
)]
async fn sandbox(
    config: &ToolConfig,
    parent: &str,
    source: &str,
    cancel: CancellationToken,
    deadline: Option<Instant>,
    output_tokens: usize,
    started: Instant,
) -> Result<AgentToolResult, ToolError> {
    use futures::stream::{FuturesUnordered, StreamExt};
    use std::cell::RefCell;
    use std::rc::Rc;
    let rt = AsyncRuntime::new().map_err(|e| ToolError::new(e.to_string()))?;
    rt.set_memory_limit(256 * 1024 * 1024).await;
    rt.set_max_stack_size(1024 * 1024).await;
    let interrupt = cancel.clone();
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let interrupt_running = running.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        interrupt_running.load(std::sync::atomic::Ordering::Relaxed)
            && (interrupt.is_cancelled() || deadline.is_some_and(|d| Instant::now() >= d))
    })))
    .await;
    let vm = AsyncContext::full(&rt)
        .await
        .map_err(|e| ToolError::new(e.to_string()))?;
    let catalog = config.context.codemode_catalog();
    let store = config.context.codemode_store();
    let models = json!({"all":config.context.codemode_models(false),"available":config.context.codemode_models(true)});
    let result = async_with!(vm => |ctx| {
        let queue = Rc::new(RefCell::new(Vec::new()));
        let requests = queue.clone();
        let dispatch = make_dispatch(&ctx, requests).map_err(|e| ToolError::new(e.to_string()))?;
        let factory: Function = ctx.eval(BOOTSTRAP).catch(&ctx).map_err(|e| ToolError::new(e.to_string()))?;
        let api: rquickjs::Object = factory.call((dispatch, serde_json::to_string(&catalog).unwrap_or_default(),
            serde_json::to_string(&store).unwrap_or_default(), models.to_string())).catch(&ctx).map_err(|e| ToolError::new(e.to_string()))?;
        let run: Function = api.get("run").map_err(|e| ToolError::new(e.to_string()))?;
        let invocation = run.call::<_, Promise>((source,)).catch(&ctx);
        let mut pending = FuturesUnordered::new();
        let error = match invocation {
          Err(error) => Some(error.to_string()),
          Ok(promise) => loop {
            while ctx.execute_pending_job() {}
            if let Some(result) = promise.result::<()>() {
                // Unawaited requests never start. Remove JS handles from the opaque
                // host queue so they cannot form a Rust/QuickJS reference cycle.
                queue.borrow_mut().clear();
                break result.catch(&ctx).err().map(|e| e.to_string());
            }
            for (name, args, resolve, reject) in queue.borrow_mut().drain(..) {
                let context = Arc::clone(&config.context);
                let parent = parent.to_owned();
                let cancel = cancel.clone();
                pending.push(async move {
                    let result = match serde_json::from_str::<JsonObject>(&args) {
                        Ok(args) if name != "codemode" => context.execute_nested(parent, name, args, cancel).await,
                        Ok(_) => Err(ToolError::new("Scripts cannot invoke codemode")),
                        Err(e) => Err(ToolError::new(format!("Tool arguments must be an object: {e}"))),
                    };
                    (result, resolve, reject)
                });
            }
            if pending.is_empty() { break Some("Script awaits a promise that cannot settle (no pending tool calls)".into()); }
            let remaining = deadline.map_or(Duration::from_hours(8760), |d| d.saturating_duration_since(Instant::now()));
            let next = tokio::select! {
                () = cancel.cancelled() => { break Some("Aborted".into()); },
                () = tokio::time::sleep(remaining) => { break Some("Script deadline exceeded".into()); },
                next = pending.next() => next,
            };
            if let Some((result, resolve, reject)) = next {
                let (function, value) = match result {
                    Ok(result) if !result.is_error => (resolve, json!({"value": nested_value(&result)})),
                    Ok(result) => (reject, json!({"error": nested_value(&result)})),
                    Err(error) => (reject, json!({"error": error.to_string()})),
                };
                function.call::<_, ()>((value.to_string(),)).catch(&ctx).map_err(|e| ToolError::new(e.to_string()))?;
            }
          },
        };
        // Drop outstanding futures and cancel their nested calls before collecting output.
        drop(pending);
        queue.borrow_mut().clear();
        cancel.cancel();
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        let snapshot: Function = api.get("snapshot").map_err(|e| ToolError::new(e.to_string()))?;
        let snapshot: String = snapshot.call(()).catch(&ctx).map_err(|e| ToolError::new(e.to_string()))?;
        Ok::<_, ToolError>((snapshot, error))
    }).await?;
    let snapshot: JsonValue =
        serde_json::from_str(&result.0).map_err(|e| ToolError::new(e.to_string()))?;
    if result.1.is_none() && snapshot["dirty"] == true {
        let values = snapshot["store"]
            .as_object()
            .cloned()
            .ok_or_else(|| ToolError::new("Invalid script store"))?;
        config.context.commit_codemode_store(values)?;
    }
    let mut content = Vec::new();
    let items = snapshot["items"].as_array().cloned().unwrap_or_default();
    let text_count = items.iter().filter(|v| v["type"] == "text").count();
    let mut text_index = 0;
    for item in items {
        if item["type"] == "image" {
            let data = item["data"].as_str().unwrap_or_default();
            let mime = item["mimeType"].as_str().unwrap_or_default();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| ToolError::new(format!("Invalid image: {e}")))?;
            let path =
                output_file(config, &bytes, mime.rsplit('/').next().unwrap_or("img")).await?;
            content.push(UserContent::text(format!(
                "Image saved to {}",
                path.display()
            )));
            content.push(UserContent::Image(oino_ai::ImageContent {
                data: data.to_owned(),
                mime_type: mime.to_owned(),
            }));
        } else {
            text_index += 1;
            let text = item["text"].as_str().unwrap_or_default();
            content.push(UserContent::text(if text_count > 1 {
                format!("==> text {text_index}/{text_count} <==\n{text}")
            } else {
                text.to_owned()
            }));
        }
    }
    if let Some(console) = snapshot["console"].as_array().filter(|v| !v.is_empty()) {
        content.push(UserContent::text(format!(
            "<console_output>\n{}\n</console_output>",
            console
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        )));
    }
    let mut full = content
        .iter()
        .filter_map(|v| match v {
            UserContent::Text(t) => Some(t.text.as_str()),
            UserContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(error) = &result.1 {
        let _ = write!(full, "\nScript error: {error}");
    }
    let budget = output_tokens.saturating_mul(4);
    if utf16_len(&full) > budget {
        let path = output_file(config, full.as_bytes(), "log").await?;
        let half = budget / 2;
        let abbreviated = format!(
            "{}\n[output omitted; full output: {}]\n{}",
            utf16_prefix(&full, half),
            path.display(),
            full.chars()
                .rev()
                .take(half)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        content.retain(|v| matches!(v, UserContent::Image(_)));
        content.insert(0, UserContent::text(abbreviated));
    } else if result.1.is_some() {
        content.push(UserContent::text(format!(
            "Script error: {}",
            result.1.as_deref().unwrap_or_default()
        )));
    }
    content.insert(
        0,
        UserContent::text(format!(
            "Script {} ({:.3}s)",
            if result.1.is_some() {
                "failed"
            } else {
                "completed"
            },
            started.elapsed().as_secs_f64()
        )),
    );
    Ok(AgentToolResult {
        content,
        is_error: result.1.is_some(),
        ..AgentToolResult::default()
    })
}

fn nested_value(result: &AgentToolResult) -> JsonValue {
    result.structured_content.clone().unwrap_or_else(|| {
        if let [UserContent::Text(text)] = result.content.as_slice() {
            JsonValue::String(text.text.clone())
        } else {
            serde_json::to_value(&result.content).unwrap_or(JsonValue::Null)
        }
    })
}

async fn output_file(
    config: &ToolConfig,
    bytes: &[u8],
    extension: &str,
) -> Result<std::path::PathBuf, ToolError> {
    let dir = std::path::Path::new(&config.agent_dir).join("tmp");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ToolError::new(e.to_string()))?;
    let path = dir.join(format!(
        "oino-codemode-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| ToolError::new(e.to_string()))?;
    Ok(path)
}

const BOOTSTRAP: &str = r"
(function(dispatch, catalogJSON, storeJSON, modelsJSON) {
  'use strict';
  const parseJSON = JSON.parse.bind(JSON), stringifyJSON = JSON.stringify.bind(JSON);
  const catalog = parseJSON(catalogJSON), values = Object.assign(Object.create(null), parseJSON(storeJSON));
  const modelCatalog = parseJSON(modelsJSON);
  const modelType = model => model.type ?? 'chat';
  const filterModels = (list,type,provider) => list.filter(m => modelType(m) === type && (!provider || m.provider === provider));
  const items = [], logs = []; let chars = 0, calls = 0, dirty = false;
  const stringify = value => typeof value === 'string' ? value : stringifyJSON(value);
  function account(value) {
    if (++calls > 100000 || (chars += value.length) > 16777216) throw Error('Script output limit exceeded');
  }
  const text = value => { const s = stringify(value) ?? 'undefined'; account(s); items.push({type:'text', text:s}); };
  const image = value => {
    if (value && value.image_url) value = value.image_url;
    if (typeof value === 'string') {
      const match = /^data:(image\/(?:png|jpeg|gif|webp));base64,([A-Za-z0-9+/=\s]+)$/.exec(value);
      if (!match) throw Error('image() requires a supported base64 data URL');
      value = {type:'image', mimeType:match[1], data:match[2]};
    }
    if (!value || !/^image\/(png|jpeg|gif|webp)$/.test(value.mimeType) || typeof value.data !== 'string') throw Error('Invalid image');
    account(value.data); items.push({type:'image', mimeType:value.mimeType, data:value.data});
  };
  const tools = Object.create(null), aliases = new Map();
  for (const entry of catalog) {
    const alias = entry.name.replace(/[^a-zA-Z0-9_$]/g, '_');
    if (aliases.has(alias)) aliases.set(alias, null); else aliases.set(alias, entry.name);
  }
  for (const [alias, name] of aliases) if (name && name !== 'codemode') {
    tools[alias] = async args => {
      if (!args || typeof args !== 'object' || Array.isArray(args)) throw Error('Tool arguments must be an object');
      const response = parseJSON(await dispatch(name, stringifyJSON(args)));
      if ('error' in response) throw Error(typeof response.error === 'string' ? response.error : stringifyJSON(response.error));
      return response.value;
    };
  }
  const store = (key, value) => {
    if (typeof key !== 'string') throw Error('store key must be a string');
    const next = Object.assign(Object.create(null), values);
    if (value === undefined) delete next[key];
    else { const s = stringifyJSON(value); if (s.length > 262144) throw Error('Store value exceeds 262144 characters'); next[key] = parseJSON(s); }
    if (stringifyJSON(next).length > 1048576) throw Error('Store exceeds 1048576 characters');
    if (value === undefined) delete values[key]; else values[key] = next[key];
    dirty = true;
  };
  const typeOf = schema => {
    if (!schema) return 'unknown';
    if (schema.enum) return schema.enum.map(v => stringifyJSON(v)).join(' | ');
    if (Array.isArray(schema.type)) return schema.type.map(type => typeOf({...schema,type})).join(' | ');
    if (schema.type === 'object') return '{ ' + Object.entries(schema.properties ?? {}).map(([key,value]) => stringifyJSON(key) + ((schema.required ?? []).includes(key) ? '' : '?') + ': ' + typeOf(value)).join('; ') + ' }';
    if (schema.type === 'array') return '(' + typeOf(schema.items) + ')[]';
    return {string:'string',number:'number',integer:'number',boolean:'boolean',null:'null'}[schema.type] ?? 'unknown';
  };
  const describe = t => ({name:t.name, description:[t.description,...(t.promptGuidelines ?? [])].join('\n'), declaration:`declare function ${t.name.replace(/[^a-zA-Z0-9_$]/g,'_')}(args: ${typeOf(t.parameters)}): Promise<unknown>;`, parameters:t.parameters, promptGuidelines:t.promptGuidelines});
  const exitToken = Object.freeze({});
  Object.assign(globalThis, {
    tools: Object.freeze(tools), text, image,
    models: Object.freeze({
      getModelsOfType: async (type,provider) => filterModels(modelCatalog.all,type,provider),
      getAvailableOfType: async (type,provider) => filterModels(modelCatalog.available,type,provider),
      getModelOfType: async (type,provider,id) => filterModels(modelCatalog.all,type,provider).find(m => m.id === id),
      classify: async model => ({provider:model.provider,model:model.id,answers:{},stopReason:'error',errorMessage:'No classifier model runtime is registered in this host'}),
      generateImages: async model => ({provider:model.provider,model:model.id,output:[],stopReason:'error',errorMessage:'No image model runtime is registered in this host'})
    }),
    console: Object.freeze(Object.fromEntries(['log','info','warn','error','debug'].map(name => [name, (...args) => { const s = args.map(stringify).join(' '); account(s); logs.push(s); }]))),
    store, load: key => Object.hasOwn(values, key) ? parseJSON(stringifyJSON(values[key])) : undefined,
    exit: () => { throw exitToken; },
    ALL_TOOLS: catalog.map(t => Object.freeze({name:t.name,description:describe(t).description})),
    searchTools: async (query, options = {}) => {
      const words = query.toLowerCase().split(/\s+/).filter(Boolean);
      return catalog.filter(t => !options.namespace || (t.namespace && (t.namespace.name ?? t.namespace) === options.namespace))
        .map(t => ({t, score:words.reduce((n,w) => n + (t.name.toLowerCase().includes(w) ? 3 : 0) + (t.description.toLowerCase().includes(w) ? 1 : 0),0)}))
        .filter(t => t.score > 0).sort((a,b) => b.score-a.score || a.t.name.localeCompare(b.t.name)).slice(0,options.limit ?? 8).map(({t}) => ({name:t.name,description:t.description}));
    },
    describeTool: async name => { const t = catalog.find(t => t.name === name); return t ? describe(t) : undefined; },
    describeNamespace: async name => { const ts = catalog.filter(t => t.namespace && (t.namespace.name ?? t.namespace) === name); return ts.length ? {name,tools:ts} : undefined; }
  });
  const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor;
  return {
    run: async source => { try { const value = await new AsyncFunction(source)(); if (value !== undefined) text(value); } catch(e) { if (e !== exitToken) throw e; } },
    snapshot: () => stringifyJSON({items,console:logs,store:values,dirty})
  };
})
";
