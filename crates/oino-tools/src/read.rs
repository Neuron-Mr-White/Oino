//! `read`.

use std::fmt::Write as _;

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{ImageContent, JsonValue, UserContent};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;
use crate::edit::storage::{TextFile, observe};
use crate::env::AccessMode;
use crate::image::{
    IMAGE_TYPE_SNIFF_BYTES, ProcessedImage, detect_supported_image_mime_type, process_image,
};
use crate::info::read_info;
use crate::js::number_to_string;
use crate::path_utils::resolve_read_path;
use crate::truncate::{
    DEFAULT_MAX_BYTES, TruncatedBy, TruncationOptions, format_size, truncate_head,
};
use crate::{aborted, args};

/// Note appended when the current model cannot see images.
pub const NON_VISION_IMAGE_NOTE: &str =
    "[Current model does not support images. The image will be omitted from this request.]";

/// The `read` tool.
#[derive(Debug)]
pub struct ReadTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl ReadTool {
    /// A `read` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: read_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for ReadTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: oino_ai::JsonObject) -> oino_ai::JsonObject {
        crate::legacy::normalize_call("read", &raw, &self.config.effective_cwd())
            .map_or(raw, |call| call.args)
    }

    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        async move {
            if cancel.is_cancelled() {
                return Err(aborted());
            }
            tokio::select! {
                result = run(&config, &call) => result,
                () = cancel.cancelled() => Err(aborted()),
            }
        }
        .boxed()
    }
}

/// JavaScript `ToIntegerOrInfinity` + `Array.prototype.slice` index clamping.
fn slice_index(value: f64, len: usize) -> usize {
    let len_f = crate::js::count_f64(len);
    let integer = if value.is_nan() { 0.0 } else { value.trunc() };
    let clamped = if integer < 0.0 {
        (len_f + integer).max(0.0)
    } else {
        integer.min(len_f)
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=len"
    )]
    let index = clamped as usize;
    index
}

fn js_slice<'a>(lines: &[&'a str], start: f64, end: f64) -> Vec<&'a str> {
    let from = slice_index(start, lines.len());
    let to = slice_index(end, lines.len());
    if from >= to {
        Vec::new()
    } else {
        lines[from..to].to_vec()
    }
}

#[allow(clippy::too_many_lines, reason = "single tool execution flow")]
async fn run(config: &ToolConfig, call: &ToolInvocation) -> Result<AgentToolResult, ToolError> {
    let canonical = crate::canonical_args("read", &call.args)?;
    let path = crate::legacy::file_path(&canonical)?;
    let legacy = call.args.contains_key("path");
    if !legacy && !crate::node_path::is_absolute(path) {
        return Err(ToolError::new("file_path must be absolute"));
    }
    for key in ["offset", "limit"] {
        if let Some(value) = call.args.get(key)
            && value.as_u64().is_none_or(|number| number == 0)
        {
            return Err(ToolError::new(format!("{key} must be a positive integer")));
        }
    }
    let offset = args::optional_f64(&call.args, "offset");
    let limit = args::optional_f64(&call.args, "limit");
    let env = config.env.as_ref();
    let absolute = resolve_read_path(env, path, &config.effective_cwd())
        .await
        .map_err(|error| ToolError::new(error.to_string()))?;
    env.access(&absolute, AccessMode::Read)
        .await
        .map_err(|error| ToolError::new(error.to_string()))?;
    let prefix = env
        .read_prefix(&absolute, IMAGE_TYPE_SNIFF_BYTES)
        .await
        .map_err(|error| ToolError::new(error.to_string()))?;
    let size = env
        .stat(&absolute)
        .await
        .map_err(|error| ToolError::new(error.to_string()))?
        .size;
    if prefix.starts_with(b"%PDF-") && size > 3 * 1024 * 1024 {
        return Err(ToolError::new("PDF exceeds the 3 MB limit"));
    }
    if detect_supported_image_mime_type(&prefix).is_some() && size > 5 * 1024 * 1024 {
        return Err(ToolError::new("Image exceeds the 5 MB limit"));
    }
    let model = config.context.model_images();
    let non_vision_note = model
        .as_ref()
        .filter(|model| !model.accepts_images)
        .map(|_| NON_VISION_IMAGE_NOTE);
    let buffer = env
        .read_file(&absolute)
        .await
        .map_err(|error| ToolError::new(error.to_string()))?;
    observe(config, &absolute, &buffer)?;
    let (content, details) = if buffer.starts_with(b"%PDF-") {
        if buffer.len() > 3 * 1024 * 1024 {
            return Err(ToolError::new("PDF exceeds the 3 MB limit"));
        }
        if !env.is_local() {
            return Err(ToolError::new(
                "PDF extraction requires a local execution environment",
            ));
        }
        let output = tokio::process::Command::new("pdftotext")
            .args(["-layout", &absolute, "-"])
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| {
                ToolError::new(format!("PDF extraction requires pdftotext: {error}"))
            })?;
        if !output.status.success() {
            return Err(ToolError::new("Could not extract PDF content"));
        }
        let (text, details) = paged_text(
            &output.stdout,
            path,
            offset,
            limit,
            args::flag(&call.args, "line_numbers"),
        )?;
        (vec![UserContent::text(text)], details)
    } else if let Some(mime) = detect_supported_image_mime_type(&prefix) {
        if buffer.len() > 5 * 1024 * 1024 {
            return Err(ToolError::new("Image exceeds the 5 MB limit"));
        }
        let high = args::optional_str(&call.args, "image_quality") == Some("high");
        let quality = oino_ai::ImageResize {
            max_width: Some(if high { 2048 } else { 1024 }),
            max_height: Some(if high { 2048 } else { 1024 }),
            max_bytes: Some(if high { 1024 * 1024 } else { 200 * 1024 }),
            jpeg_quality: Some(if high { 90 } else { 75 }),
        };
        let resize = model
            .as_ref()
            .and_then(|model| model.resize.clone())
            .or_else(|| config.images.fallback_resize.clone())
            .or(Some(quality));
        (
            image_content(
                &buffer,
                mime,
                config.images.auto_resize,
                resize.as_ref(),
                non_vision_note,
            ),
            None,
        )
    } else {
        let (text, details) = if legacy {
            text_content(&buffer, path, offset, limit)?
        } else {
            paged_text(
                &buffer,
                path,
                offset,
                limit,
                args::flag(&call.args, "line_numbers"),
            )?
        };
        (vec![UserContent::text(text)], details)
    };
    let structured = structured_output(&content);
    Ok(AgentToolResult {
        content,
        details,
        structured_content: Some(structured),
        ..AgentToolResult::default()
    })
}

fn paged_text(
    buffer: &[u8],
    path: &str,
    offset: Option<f64>,
    limit: Option<f64>,
    numbered: bool,
) -> Result<(String, Option<JsonValue>), ToolError> {
    let decoded = TextFile::decode(buffer)?;
    let text = decoded.text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.lines().collect();
    let start = usize::try_from(args::to_count(offset.unwrap_or(1.0)))
        .unwrap_or(usize::MAX)
        .saturating_sub(1);
    let count = usize::try_from(args::to_count(limit.unwrap_or(2400.0)))
        .unwrap_or(2400)
        .min(2400);
    if start >= lines.len() && start > 0 {
        return Err(ToolError::new(format!(
            "Offset {} is beyond end of file ({} lines total)",
            start + 1,
            lines.len()
        )));
    }
    if offset.is_none() && limit.is_none() && lines.len() > 2400 {
        let declaration = regex::Regex::new(r"^\s*(?:(?:pub(?:\([^)]*\))?|export|async|static|abstract|public|private|protected)\s+)*(?:fn|struct|enum|trait|impl|mod|type|const|function|class|interface|def|async def)\b")
            .map_err(|error| ToolError::new(error.to_string()))?;
        let outline = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| declaration.is_match(line))
            .map(|(index, line)| format!("{}→{line}", index + 1))
            .collect::<Vec<_>>();
        if !outline.is_empty() {
            let truncated = truncate_head(&outline.join("\n"), TruncationOptions::bytes_only());
            return Ok((
                format!(
                    "[Declaration outline of {path} ({} lines). Use offset and limit to read the body.]\n{}",
                    lines.len(),
                    truncated.content
                ),
                None,
            ));
        }
    }
    let end = start.saturating_add(count).min(lines.len());
    let mut result = String::new();
    let mut next = start;
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        let shown = if numbered {
            format!("{}→{line}", index + 1)
        } else {
            (*line).to_owned()
        };
        if result.len() + shown.len() > usize::try_from(DEFAULT_MAX_BYTES).unwrap_or(usize::MAX) {
            if next == start {
                return Err(ToolError::new(format!(
                    "Line {} exceeds the read output byte limit",
                    index + 1
                )));
            }
            break;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&shown);
        next = index + 1;
    }
    if next < lines.len() {
        let _ = write!(
            result,
            "\n\n[Showing lines {}-{} of {}. Use offset={} to continue.]",
            start + 1,
            next,
            lines.len(),
            next + 1
        );
    }
    Ok((result, None))
}

fn image_content(
    buffer: &[u8],
    mime: &str,
    auto_resize: bool,
    resize: Option<&oino_ai::ImageResize>,
    non_vision_note: Option<&str>,
) -> Vec<UserContent> {
    match process_image(buffer, mime, auto_resize, resize) {
        ProcessedImage::Omitted { message } => {
            let mut note = format!("Read image file [{mime}]\n{message}");
            if let Some(extra) = non_vision_note {
                note.push('\n');
                note.push_str(extra);
            }
            vec![UserContent::text(note)]
        }
        ProcessedImage::Ok {
            data,
            mime_type,
            hints,
        } => {
            let mut note = format!("Read image file [{mime_type}]");
            if !hints.is_empty() {
                note.push('\n');
                note.push_str(&hints.join("\n"));
            }
            if let Some(extra) = non_vision_note {
                note.push('\n');
                note.push_str(extra);
            }
            vec![
                UserContent::text(note),
                UserContent::Image(ImageContent { data, mime_type }),
            ]
        }
    }
}

fn text_content(
    buffer: &[u8],
    path: &str,
    offset: Option<f64>,
    limit: Option<f64>,
) -> Result<(String, Option<JsonValue>), ToolError> {
    let text = String::from_utf8_lossy(buffer);
    let all_lines: Vec<&str> = text.split('\n').collect();
    let total = crate::js::count_f64(all_lines.len());
    let start_line = match offset {
        Some(offset) if offset != 0.0 && !offset.is_nan() => (offset - 1.0).max(0.0),
        _ => 0.0,
    };
    let start_display = start_line + 1.0;
    if start_line >= total {
        let shown = offset.map_or_else(|| "undefined".to_owned(), number_to_string);
        return Err(ToolError::new(format!(
            "Offset {shown} is beyond end of file ({} lines total)",
            all_lines.len()
        )));
    }
    let (selected, user_limited) = match limit {
        Some(limit) => {
            let end = (start_line + limit).min(total);
            (
                js_slice(&all_lines, start_line, end).join("\n"),
                Some(end - start_line),
            )
        }
        None => (js_slice(&all_lines, start_line, total).join("\n"), None),
    };
    let truncation = truncate_head(&selected, TruncationOptions::default());
    let max = format_size(DEFAULT_MAX_BYTES);
    if truncation.first_line_exceeds_limit {
        let first = all_lines
            .get(slice_index(start_line, all_lines.len()))
            .copied()
            .unwrap_or("");
        let shown = number_to_string(start_display);
        let output = format!(
            "[Line {shown} is {}, exceeds {max} limit. Use bash: sed -n '{shown}p' {path} | head -c {DEFAULT_MAX_BYTES}]",
            format_size(first.len() as u64)
        );
        return Ok((output, Some(json!({ "truncation": truncation }))));
    }
    if truncation.truncated {
        #[allow(clippy::cast_precision_loss, reason = "line counts are far below 2^53")]
        let end_display = start_display + truncation.output_lines as f64 - 1.0;
        let next = number_to_string(end_display + 1.0);
        let (from, to) = (
            number_to_string(start_display),
            number_to_string(end_display),
        );
        let notice = if truncation.truncated_by == Some(TruncatedBy::Lines) {
            format!(
                "\n\n[Showing lines {from}-{to} of {}. Use offset={next} to continue.]",
                all_lines.len()
            )
        } else {
            format!(
                "\n\n[Showing lines {from}-{to} of {} ({max} limit). Use offset={next} to continue.]",
                all_lines.len()
            )
        };
        let output = format!("{}{notice}", truncation.content);
        return Ok((output, Some(json!({ "truncation": truncation }))));
    }
    if let Some(limited) = user_limited
        && start_line + limited < total
    {
        let remaining = number_to_string(total - (start_line + limited));
        let next = number_to_string(start_line + limited + 1.0);
        return Ok((
            format!(
                "{}\n\n[{remaining} more lines in file. Use offset={next} to continue.]",
                truncation.content
            ),
            None,
        ));
    }
    Ok((truncation.content, None))
}

/// The text, or `{type:"image", data, mimeType, note}`.
fn structured_output(content: &[UserContent]) -> JsonValue {
    let text = content
        .iter()
        .find_map(|block| match block {
            UserContent::Text(text) => Some(text.text.clone()),
            UserContent::Image(_) => None,
        })
        .unwrap_or_default();
    let image = content.iter().find_map(|block| match block {
        UserContent::Image(image) => Some(image),
        UserContent::Text(_) => None,
    });
    match image {
        Some(image) => {
            json!({"type": "image", "data": image.data, "mimeType": image.mime_type, "note": text})
        }
        None => JsonValue::String(text),
    }
}

/// Text that replaces images when `images.blockImages` is set (the `convertToLlm` wrapper).
pub const IMAGE_READING_DISABLED: &str = "Image reading is disabled.";

/// The `blockImages` filter: every image becomes [`IMAGE_READING_DISABLED`], consecutive
/// copies of that text collapse into one. Core applies it when converting messages for the
/// model; the tool itself always returns images.
#[must_use]
pub fn block_images(content: &[UserContent]) -> Vec<UserContent> {
    let mut out: Vec<UserContent> = Vec::with_capacity(content.len());
    for block in content {
        let block = match block {
            UserContent::Image(_) => UserContent::text(IMAGE_READING_DISABLED),
            UserContent::Text(_) => block.clone(),
        };
        let duplicate = matches!(
            (&block, out.last()),
            (UserContent::Text(current), Some(UserContent::Text(previous)))
                if current.text == IMAGE_READING_DISABLED && previous.text == IMAGE_READING_DISABLED
        );
        if !duplicate {
            out.push(block);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_matches_javascript() {
        let lines = ["a", "b", "c"];
        assert_eq!(js_slice(&lines, 0.0, -1.0), vec!["a", "b"]);
        assert_eq!(js_slice(&lines, 1.5, 3.0), vec!["b", "c"]);
        assert_eq!(js_slice(&lines, 2.0, 1.0), Vec::<&str>::new());
    }

    #[test]
    fn block_images_dedupes() {
        let image = UserContent::Image(ImageContent {
            data: String::new(),
            mime_type: "image/png".into(),
        });
        let out = block_images(&[image.clone(), image, UserContent::text("x")]);
        assert_eq!(
            out,
            vec![
                UserContent::text(IMAGE_READING_DISABLED),
                UserContent::text("x")
            ]
        );
    }
}
