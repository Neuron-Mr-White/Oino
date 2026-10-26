//! Self-contained HTML export of a session branch (pi `session-export.ts` structure: header,
//! messages, tool calls, collapsed thinking, embedded CSS, no external assets).

use std::fmt::Write as _;

use oino_ai::{AgentMessage, AssistantContent, UserContent};
use oino_session::{EntryKind, SessionEntry};

const CSS: &str = "body{font-family:system-ui,sans-serif;max-width:960px;margin:2rem auto;padding:0 1rem;background:#fafafa;color:#222}\
header{border-bottom:1px solid #ddd;margin-bottom:1rem}\
.msg{border-radius:8px;padding:.75rem 1rem;margin:.75rem 0;background:#fff;border:1px solid #e5e5e5}\
.user{border-left:4px solid #3b82f6}.assistant{border-left:4px solid #10b981}.tool{border-left:4px solid #f59e0b}\
.meta{font-size:.8rem;color:#666;margin-bottom:.25rem}pre{white-space:pre-wrap;word-break:break-word;margin:0}\
details{margin:.25rem 0}summary{cursor:pointer;color:#555}.error{color:#b91c1c}";

/// Escapes text for HTML.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn block(out: &mut String, class: &str, meta: &str, body: &str) {
    let _ = write!(
        out,
        "<div class=\"msg {class}\"><div class=\"meta\">{}</div>{body}</div>",
        escape_html(meta)
    );
}

fn pre(text: &str) -> String {
    format!("<pre>{}</pre>", escape_html(text))
}

fn user_blocks(content: &[UserContent]) -> String {
    content
        .iter()
        .map(|block| match block {
            UserContent::Text(text) => pre(&text.text),
            UserContent::Image(image) => format!(
                "<img alt=\"image\" style=\"max-width:100%\" src=\"data:{};base64,{}\">",
                escape_html(&image.mime_type),
                escape_html(&image.data)
            ),
        })
        .collect()
}

/// Renders the branch as an HTML document.
#[must_use]
pub fn render_html(session_id: &str, name: Option<&str>, branch: &[SessionEntry]) -> String {
    let title = name.unwrap_or(session_id);
    let mut out = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title><style>{CSS}</style></head><body><header><h1>{}</h1><p class=\"meta\">Session {}</p></header>",
        escape_html(title),
        escape_html(title),
        escape_html(session_id)
    );
    for entry in branch {
        match &entry.kind {
            EntryKind::Message { message } => match message {
                AgentMessage::User(user) => block(
                    &mut out,
                    "user",
                    "User",
                    &user_blocks(&user.content.clone().into_blocks()),
                ),
                AgentMessage::Assistant(assistant) => {
                    let mut body = String::new();
                    for content in &assistant.content {
                        match content {
                            AssistantContent::Text(text) => body.push_str(&pre(&text.text)),
                            AssistantContent::Thinking(thinking) => {
                                let _ = write!(
                                    body,
                                    "<details><summary>Thinking</summary>{}</details>",
                                    pre(&thinking.thinking)
                                );
                            }
                            AssistantContent::ToolCall(call) => {
                                let args = serde_json::to_string_pretty(&call.arguments)
                                    .unwrap_or_default();
                                let _ = write!(
                                    body,
                                    "<details open><summary>{}</summary>{}</details>",
                                    escape_html(&call.name),
                                    pre(&args)
                                );
                            }
                        }
                    }
                    if let Some(error) = &assistant.error_message {
                        let _ = write!(body, "<p class=\"error\">{}</p>", escape_html(error));
                    }
                    block(
                        &mut out,
                        "assistant",
                        &format!("Assistant · {}/{}", assistant.provider, assistant.model),
                        &body,
                    );
                }
                AgentMessage::ToolResult(result) => block(
                    &mut out,
                    "tool",
                    &format!(
                        "{}{}",
                        result.tool_name,
                        if result.is_error { " (error)" } else { "" }
                    ),
                    &user_blocks(&result.content),
                ),
                AgentMessage::BashExecution(bash) => block(
                    &mut out,
                    "tool",
                    &format!("$ {}", bash.command),
                    &pre(&bash.output),
                ),
                AgentMessage::Custom(custom) if custom.display => {
                    block(
                        &mut out,
                        "tool",
                        &custom.custom_type,
                        &user_blocks(&custom.content.clone().into_blocks()),
                    );
                }
                _ => {}
            },
            EntryKind::Compaction(compaction) => {
                block(&mut out, "tool", "Compaction", &pre(&compaction.summary));
            }
            EntryKind::BranchSummary(summary) => {
                block(&mut out, "tool", "Branch summary", &pre(&summary.summary));
            }
            _ => {}
        }
    }
    out.push_str("</body></html>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup() {
        assert_eq!(
            escape_html("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
        let html = render_html("abc", None, &[]);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>abc</title>"));
    }
}
