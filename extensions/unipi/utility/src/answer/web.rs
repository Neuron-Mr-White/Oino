//! `/unipi:answer web` (unipi `src/answer/web.ts`): a one-shot local page with the agent's
//! full reply on the left and one answer box per question on the right (plus a free-text
//! note). Served through the host loopback listener (`http.listener`, 127.0.0.1) behind a
//! random token in the URL.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

/// Fixed port tried first over SSH, so one `ssh -L` keeps working.
pub const SSH_DEFAULT_PORT: u16 = 47_321;
/// Largest accepted POST body.
pub const MAX_BODY: usize = 1_000_000;

/// Submitted answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebAnswer {
    /// One answer per question (in order).
    pub answers: Vec<String>,
    /// Free-text note.
    pub note: String,
}

/// True when any SSH variable is set (`SSH_CONNECTION`, `SSH_TTY`, `SSH_CLIENT`).
#[must_use]
pub fn is_ssh(get: &dyn Fn(&str) -> Option<String>) -> bool {
    ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|k| get(k).is_some_and(|v| !v.is_empty()))
}

/// Port to bind: the setting, else 47321 over SSH, else any (0).
#[must_use]
pub fn choose_port(setting: u16, ssh: bool) -> u16 {
    match (setting, ssh) {
        (p, _) if p > 0 => p,
        (_, true) => SSH_DEFAULT_PORT,
        _ => 0,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

static CODE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"`([^`]+)`").ok());
static BOLD: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").ok());

/// `` `code` `` and `**bold**` after escaping.
fn inline(s: &str) -> String {
    let e = esc(s);
    let e = CODE.as_ref().map_or_else(
        || e.clone(),
        |re| re.replace_all(&e, "<code>$1</code>").into_owned(),
    );
    BOLD.as_ref().map_or_else(
        || e.clone(),
        |re| re.replace_all(&e, "<b>$1</b>").into_owned(),
    )
}

fn table_html(rows: &[Vec<String>]) -> String {
    let Some((head, body)) = rows.split_first() else {
        return String::new();
    };
    let th = head
        .iter()
        .map(|c| format!("<th>{}</th>", inline(c)))
        .collect::<Vec<_>>()
        .concat();
    let tb = body
        .iter()
        .map(|r| {
            let tds = r
                .iter()
                .map(|c| format!("<td>{}</td>", inline(c)))
                .collect::<Vec<_>>()
                .concat();
            format!("<tr>{tds}</tr>")
        })
        .collect::<Vec<_>>()
        .concat();
    format!("<table><thead><tr>{th}</tr></thead><tbody>{tb}</tbody></table>")
}

fn list_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix(['-', '*', '+']) {
        return rest.strip_prefix(char::is_whitespace).map(str::trim_start);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = t[digits..].strip_prefix(['.', ')'])?;
    rest.strip_prefix(char::is_whitespace).map(str::trim_start)
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line[hashes..].strip_prefix(char::is_whitespace)?;
    Some((hashes, rest.trim_start()))
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 2 && t.starts_with('|') && t.ends_with('|')
}

fn is_separator_row(line: &str) -> bool {
    let t = line.trim();
    is_table_row(line)
        && t[1..t.len() - 1]
            .chars()
            .all(|c| matches!(c, ' ' | ':' | '|' | '-' | '\t'))
        && t.len() > 2
}

/// Tiny markdown → HTML for the reply pane: fences, headings, lists, tables, bold, code.
#[must_use]
pub fn render_reply(md: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let mut list = false;
    let mut table: Option<Vec<Vec<String>>> = None;
    for line in md.split('\n') {
        if code.is_none() && is_table_row(line) {
            if is_separator_row(line) {
                continue;
            }
            let t = line.trim();
            let cells = t[1..t.len() - 1]
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect();
            table.get_or_insert_with(Vec::new).push(cells);
            continue;
        }
        if let Some(rows) = table.take() {
            out.push(table_html(&rows));
        }
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(lines) => out.push(format!(
                    "<pre><code>{}</code></pre>",
                    esc(&lines.join("\n"))
                )),
                None => code = Some(Vec::new()),
            }
            continue;
        }
        if let Some(lines) = code.as_mut() {
            lines.push(line);
            continue;
        }
        if let Some(item) = list_item(line) {
            if !list {
                out.push("<ul>".into());
            }
            list = true;
            out.push(format!("<li>{}</li>", inline(item)));
            continue;
        }
        if list {
            out.push("</ul>".into());
            list = false;
        }
        if let Some((level, text)) = heading(line) {
            let h = (level + 1).min(4);
            out.push(format!("<h{h}>{}</h{h}>", inline(text)));
        } else if !line.trim().is_empty() {
            out.push(format!("<p>{}</p>", inline(line)));
        }
    }
    if let Some(rows) = table.take() {
        out.push(table_html(&rows));
    }
    if let Some(lines) = code {
        out.push(format!(
            "<pre><code>{}</code></pre>",
            esc(&lines.join("\n"))
        ));
    }
    if list {
        out.push("</ul>".into());
    }
    out.join("\n")
}

const STYLE: &str = r":root{color-scheme:dark;--bg:#16181d;--panel:#1e2128;--line:#2c313a;--fg:#d7dae0;--dim:#8a919e;--accent:#53a0d7}
  *{box-sizing:border-box} body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.55 system-ui,sans-serif}
  main{display:grid;grid-template-columns:minmax(0,1.1fr) minmax(0,1fr);height:100vh}
  section{overflow:auto;padding:24px 28px} .reply{border-right:1px solid var(--line)}
  h1{font-size:13px;letter-spacing:.08em;text-transform:uppercase;color:var(--dim);margin:0 0 16px}
  table{border-collapse:collapse;margin:10px 0} th,td{border:1px solid var(--line);padding:5px 10px;text-align:left} th{color:var(--dim);font-weight:600}
  pre{background:var(--panel);padding:10px 12px;border-radius:6px;overflow:auto} code{font:12.5px ui-monospace,monospace}
  label{display:flex;gap:10px;margin:18px 0 6px} .n{color:var(--accent);font-weight:600} .q{flex:1}
  textarea{width:100%;background:var(--panel);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:8px 10px;font:inherit;resize:vertical}
  textarea:focus{outline:none;border-color:var(--accent)}
  .bar{position:sticky;bottom:0;background:var(--bg);padding:14px 0;display:flex;gap:12px;align-items:center}
  button{background:var(--accent);color:#0b0d10;border:0;border-radius:6px;padding:9px 18px;font-weight:600;cursor:pointer}
  .hint{color:var(--dim);font-size:12px} .done{padding:40px;font-size:16px}
  @media (max-width:900px){main{grid-template-columns:1fr;height:auto}.reply{border-right:0;border-bottom:1px solid var(--line)}}";

/// The form page.
#[must_use]
pub fn form_page(reply: &str, questions: &[String], token: &str) -> String {
    let fallback = ["Your reply".to_owned()];
    let qs: &[String] = if questions.is_empty() {
        &fallback
    } else {
        questions
    };
    let fields = qs
        .iter()
        .enumerate()
        .map(|(i, q)| {
            format!(
                "\n      <label><span class=\"n\">{}</span><span class=\"q\">{}</span></label>\n      <textarea name=\"a{i}\" rows=\"3\" placeholder=\"Answer (empty = skip)\"></textarea>",
                i + 1,
                esc(q)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let keys = json!(
        qs.iter()
            .enumerate()
            .map(|(i, _)| format!("a{i}"))
            .collect::<Vec<_>>()
    );
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Answer — Oino</title>
<style>
  {STYLE}
</style></head><body><main>
  <section class="reply"><h1>Agent's reply</h1>{reply_html}</section>
  <section><h1>Your answers</h1>
    <form id="f">{fields}
      <label><span class="q">Anything else</span></label>
      <textarea name="note" rows="3" placeholder="Optional note"></textarea>
      <div class="bar"><button type="submit">Send to Oino</button><span class="hint">Ctrl+Enter sends</span></div>
    </form>
  </section>
</main>
<script>
const f=document.getElementById("f");
const send=async()=>{{const d=new FormData(f);const answers={keys}.map(k=>d.get(k)||"");
  const r=await fetch("/a/{token}",{{method:"POST",headers:{{"content-type":"application/json"}},body:JSON.stringify({{answers,note:d.get("note")||""}})}});
  document.body.innerHTML='<div class="done">'+(r.ok?"Sent to Oino. You can close this tab.":"Could not send — is Oino still waiting?")+'</div>';}};
f.addEventListener("submit",e=>{{e.preventDefault();send();}});
f.addEventListener("keydown",e=>{{if(e.key==="Enter"&&(e.ctrlKey||e.metaKey)){{e.preventDefault();send();}}}});
document.querySelector("textarea")?.focus();
</script></body></html>"#,
        reply_html = render_reply(reply),
    )
}

/// How the server answers one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `200` with the page.
    Page,
    /// `200 ok`; the form is done.
    Submitted(WebAnswer),
    /// Any other status (404 wrong path, 405 method, 400 bad body, 413 too large).
    Status(u16),
}

/// Routes one request (`/a/<token>` only; GET page, POST JSON answers).
#[must_use]
pub fn route(method: &str, path: &str, body: &[u8], token: &str) -> Reply {
    if path != format!("/a/{token}") {
        return Reply::Status(404);
    }
    match method {
        "GET" => Reply::Page,
        "POST" if body.len() > MAX_BODY => Reply::Status(413),
        "POST" => match serde_json::from_slice::<Value>(body) {
            Ok(data) => Reply::Submitted(WebAnswer {
                answers: data
                    .get("answers")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                other => other.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                note: data
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            Err(_) => Reply::Status(400),
        },
        _ => Reply::Status(405),
    }
}

/// Lines shown while the form is open.
#[must_use]
pub fn waiting_lines(url: &str, port: u16, ssh: bool, host: &str) -> Vec<String> {
    if ssh {
        vec![
            "Open on your own machine:".into(),
            url.to_owned(),
            "First forward the port from that machine:".into(),
            format!("ssh -L {port}:127.0.0.1:{port} {host}"),
        ]
    } else {
        vec!["Opened in your browser:".into(), url.to_owned()]
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn renders_markdown_tables() {
        assert!(render_reply("| A | B |\n|---|---|\n| 1 | 2 |").contains(
            "<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>"
        ));
    }

    #[test]
    fn escapes_html_and_renders_inline() {
        assert!(
            render_reply("<script>x</script> **b**")
                .contains("&lt;script&gt;x&lt;/script&gt; <b>b</b>")
        );
        assert_eq!(
            render_reply("use `a<b` now"),
            "<p>use <code>a&lt;b</code> now</p>"
        );
        assert_eq!(
            render_reply("# Title\n- one\n- two\ntext"),
            "<h2>Title</h2>\n<ul>\n<li>one</li>\n<li>two</li>\n</ul>\n<p>text</p>"
        );
        assert_eq!(
            render_reply("```\n<x>\n```"),
            "<pre><code>&lt;x&gt;</code></pre>"
        );
        assert_eq!(render_reply("```\nopen"), "<pre><code>open</code></pre>");
        assert_eq!(render_reply("a ` b"), "<p>a ` b</p>");
    }

    #[test]
    fn page_and_routing() {
        let page = form_page(
            crate::answer::extract::tests::REPLY,
            &["Start?".into(), "Where?".into()],
            "tok",
        );
        assert!(page.contains("Should I start Phase 1?"));
        assert!(page.contains("name=\"a1\""));
        assert!(page.contains("[\"a0\",\"a1\"]"));
        assert!(page.contains("/a/tok"));
        assert!(form_page("", &[], "t").contains("Your reply"));
        assert_eq!(route("GET", "/a/tok", b"", "tok"), Reply::Page);
        assert_eq!(route("GET", "/a/wrong", b"", "tok"), Reply::Status(404));
        assert_eq!(route("PUT", "/a/tok", b"", "tok"), Reply::Status(405));
        assert_eq!(route("POST", "/a/tok", b"nope", "tok"), Reply::Status(400));
        assert_eq!(
            route(
                "POST",
                "/a/tok",
                br#"{"answers":["yes","here",null],"note":"n"}"#,
                "tok"
            ),
            Reply::Submitted(WebAnswer {
                answers: vec!["yes".into(), "here".into(), String::new()],
                note: "n".into()
            })
        );
    }

    #[test]
    fn ssh_detection_and_ports() {
        assert!(is_ssh(
            &|k| (k == "SSH_CONNECTION").then(|| "1 2 3 4".to_owned())
        ));
        assert!(!is_ssh(&|_| None));
        assert_eq!(choose_port(0, true), SSH_DEFAULT_PORT);
        assert_eq!(choose_port(0, false), 0);
        assert_eq!(choose_port(8080, true), 8080);
        assert_eq!(
            waiting_lines("u", 5, true, "box")[3],
            "ssh -L 5:127.0.0.1:5 box"
        );
        assert_eq!(waiting_lines("u", 5, false, "box").len(), 2);
    }
}
