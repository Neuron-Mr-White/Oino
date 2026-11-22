//! Questions in, answers out (unipi `src/answer/extract.ts`, pure).
//!
//! Pulls the questions out of the agent's last reply (every sentence ending in `?` outside
//! code blocks, list/heading markers stripped, URLs ignored), builds a Q/A template, parses it
//! back and composes one user message that quotes each question.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// At most this many questions are extracted.
pub const MAX_QUESTIONS: usize = 20;

static MARKER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+[.)]|#{1,6}|>)\s+").ok());
static URL: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"https?://\S+").ok());
static ANSWER_LINE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^A(\d+):\s?(.*)$").ok());
static QUESTION_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^Q\d+\.\s").ok());

fn is_marker(line: &str) -> bool {
    MARKER.as_ref().is_some_and(|re| re.is_match(line))
}

/// Plain text of message content (string or `text` parts joined with newlines).
#[must_use]
pub fn message_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn clean_line(line: &str) -> String {
    let stripped = MARKER
        .as_ref()
        .map_or_else(|| line.to_owned(), |re| re.replace(line, "").into_owned());
    let no_emphasis = stripped
        .replace("**", "")
        .replace("__", "")
        .replace('`', "");
    no_emphasis.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Sentences ending in `?` (unipi's `/[^.!?]*(?:[.!](?!\s|$)[^.!?]*)*\?+/g`): a `.` or `!`
/// not followed by whitespace or the end (`e.g`, `v1.2`) stays inside a sentence.
fn question_sentences(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < c.len() {
        let mut j = start;
        let matched = loop {
            while j < c.len() && !matches!(c[j], '.' | '!' | '?') {
                j += 1;
            }
            match c.get(j) {
                Some('?') => {
                    while c.get(j) == Some(&'?') {
                        j += 1;
                    }
                    break true;
                }
                Some('.' | '!') if c.get(j + 1).is_some_and(|n| !n.is_whitespace()) => j += 1,
                _ => break false,
            }
        };
        if matched {
            out.push(c[start..j].iter().collect());
            start = j;
        } else {
            start = j + 1;
        }
    }
    out
}

/// Questions in a reply: lines joined into paragraphs (a wrapped question stays whole), list
/// items and headings start their own paragraph, code fences skipped, every sentence ending in
/// `?` becomes one question.
#[must_use]
pub fn extract_questions(text: &str) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut fenced = false;
    let flush = |current: &mut Vec<&str>, paragraphs: &mut Vec<String>| {
        if !current.is_empty() {
            paragraphs.push(current.join(" "));
            current.clear();
        }
    };
    for raw in text.split('\n') {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut current, &mut paragraphs);
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if raw.trim().is_empty() {
            flush(&mut current, &mut paragraphs);
            continue;
        }
        if is_marker(raw) {
            flush(&mut current, &mut paragraphs);
        }
        current.push(raw);
    }
    flush(&mut current, &mut paragraphs);

    let mut out: Vec<String> = Vec::new();
    for para in paragraphs {
        let clean = clean_line(&para);
        if !clean.contains('?') {
            continue;
        }
        let mut urls: Vec<String> = Vec::new();
        let masked = URL.as_ref().map_or_else(
            || clean.clone(),
            |re| {
                re.replace_all(&clean, |m: &regex::Captures<'_>| {
                    urls.push(m[0].to_owned());
                    format!("\u{0}{}\u{0}", urls.len() - 1)
                })
                .into_owned()
            },
        );
        if !masked.contains('?') {
            continue;
        }
        for sentence in question_sentences(&masked) {
            let q = unmask(&sentence, &urls).trim().to_owned();
            if q.chars().count() < 4 || out.contains(&q) {
                continue;
            }
            out.push(q);
            if out.len() >= MAX_QUESTIONS {
                return out;
            }
        }
    }
    out
}

fn unmask(s: &str, urls: &[String]) -> String {
    let mut out = String::new();
    let mut parts = s.split('\u{0}');
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    let rest: Vec<&str> = parts.collect();
    for pair in rest.chunks(2) {
        let idx = pair.first().and_then(|i| i.parse::<usize>().ok());
        out.push_str(idx.and_then(|i| urls.get(i)).map_or("", String::as_str));
        if let Some(tail) = pair.get(1) {
            out.push_str(tail);
        }
    }
    out
}

/// Q/A pairs, nothing else in the buffer; no questions → an empty buffer.
#[must_use]
pub fn build_template(questions: &[String]) -> String {
    if questions.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = questions
        .iter()
        .enumerate()
        .flat_map(|(i, q)| {
            [
                format!("Q{}. {q}", i + 1),
                format!("A{}: ", i + 1),
                String::new(),
            ]
        })
        .collect();
    format!("{} ", lines.join("\n").trim_end())
}

/// Line index of the first answer line (`A1:`), where the cursor should start.
#[must_use]
pub fn first_answer_line(template: &str) -> Option<usize> {
    template.split('\n').position(|l| l.starts_with("A1:"))
}

/// Answers by question index (0-based); missing/empty → `""`.
#[must_use]
pub fn parse_template(text: &str, count: usize) -> Vec<String> {
    if count == 0 {
        return vec![text.trim().to_owned()];
    }
    let mut buf: Vec<Vec<String>> = vec![Vec::new(); count];
    let mut current: Option<usize> = None;
    for line in text.split('\n') {
        if QUESTION_LINE.as_ref().is_some_and(|re| re.is_match(line)) {
            current = None;
            continue;
        }
        if let Some(caps) = ANSWER_LINE.as_ref().and_then(|re| re.captures(line)) {
            current = caps[1]
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .filter(|i| *i < count);
            if let Some(i) = current {
                buf[i].push(caps[2].to_owned());
            }
            continue;
        }
        if let Some(i) = current {
            buf[i].push(line.to_owned());
        }
    }
    buf.into_iter()
        .map(|l| l.join("\n").trim().to_owned())
        .collect()
}

/// One user message; `None` when nothing was answered.
#[must_use]
pub fn compose_answers(questions: &[String], answers: &[String], note: &str) -> Option<String> {
    let note = note.trim();
    if questions.is_empty() {
        let text: Vec<&str> = [answers.first().map_or("", String::as_str).trim(), note]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        return Some(text.join("\n\n")).filter(|t| !t.is_empty());
    }
    let mut parts = Vec::new();
    let mut skipped = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        let a = answers.get(i).map_or("", |a| a.trim());
        if a.is_empty() {
            skipped.push(format!("#{}", i + 1));
        } else {
            parts.push(format!("{}. > {q}\n\n{a}", i + 1));
        }
    }
    if parts.is_empty() && note.is_empty() {
        return None;
    }
    let mut lines = vec![parts.join("\n\n")];
    if !note.is_empty() {
        lines.push(note.to_owned());
    }
    if !skipped.is_empty() {
        lines.push(format!("(Not answered: {})", skipped.join(", ")));
    }
    Some(
        lines
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

/// Text of the last assistant message with text on a branch (`message` entries).
#[must_use]
pub fn last_assistant_text(entries: &[Value]) -> String {
    entries
        .iter()
        .rev()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|e| e.get("message"))
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|m| {
            message_text(m.get("content").unwrap_or(&Value::Null))
                .trim()
                .to_owned()
        })
        .find(|t| !t.is_empty())
        .unwrap_or_default()
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
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    pub(crate) const REPLY: &str = "Here is the plan.\n\n1. **Should I start Phase 1?**\n- Do you want your global skills mode changed now?\n\n```ts\nconst x = a ? b : c; // not a question?\n```\n\nSee https://example.com/page?x=1 for details.\nWhere did you see the tag? A screenshot helps.";

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn question_lines_outside_code_with_markers_stripped_and_urls_ignored() {
        assert_eq!(
            extract_questions(REPLY),
            strings(&[
                "Should I start Phase 1?",
                "Do you want your global skills mode changed now?",
                "Where did you see the tag?"
            ])
        );
    }

    #[test]
    fn reads_assistant_content_arrays() {
        let c = json!([{"type": "thinking", "thinking": "x"}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]);
        assert_eq!(message_text(&c), "a\nb");
        assert_eq!(message_text(&json!("plain")), "plain");
        assert_eq!(message_text(&Value::Null), "");
    }

    #[test]
    fn template_round_trip_keeps_multi_line_answers() {
        let qs = strings(&["Start?", "Change mode?", "Where?"]);
        let t = build_template(&qs);
        assert_eq!(
            t,
            "Q1. Start?\nA1: \n\nQ2. Change mode?\nA2: \n\nQ3. Where?\nA3: "
        );
        assert_eq!(first_answer_line(&t), Some(1));
        let filled = t
            .replacen("A1: ", "A1: yes, go\n# and commit per phase", 1)
            .replacen("A3: ", "A3: in the dva model", 1);
        assert_eq!(
            parse_template(&filled, 3),
            strings(&["yes, go\n# and commit per phase", "", "in the dva model"])
        );
        assert_eq!(
            parse_template("A9: out of range\nA2: two", 2),
            strings(&["", "two"])
        );
    }

    #[test]
    fn composes_quoted_answers_and_lists_skipped() {
        let msg = compose_answers(
            &strings(&["Start?", "Change mode?"]),
            &strings(&["yes", ""]),
            "thanks",
        );
        assert_eq!(
            msg.as_deref(),
            Some("1. > Start?\n\nyes\n\nthanks\n\n(Not answered: #2)")
        );
        assert_eq!(
            compose_answers(&strings(&["Start?"]), &strings(&[""]), ""),
            None
        );
        assert_eq!(
            compose_answers(&strings(&["Start?"]), &[], "only a note").as_deref(),
            Some("only a note\n\n(Not answered: #1)")
        );
    }

    #[test]
    fn no_questions_is_free_text() {
        assert_eq!(build_template(&[]), "");
        assert_eq!(parse_template("just do it\n", 0)[0], "just do it");
        assert_eq!(
            compose_answers(&[], &strings(&["just do it"]), "").as_deref(),
            Some("just do it")
        );
        assert_eq!(compose_answers(&[], &strings(&[""]), ""), None);
    }

    #[test]
    fn long_wrapped_questions_stay_whole_and_sentences_split() {
        let reply = "Some context first.\n\nIf you could look back from ten years in the future, what is the one thing\nyou hope you will understand about yourself, and that no amount of advice\nfrom anyone else could have handed to you?\n\n- Which port? And should I use TLS?\n\nSee https://x.dev/a?b=1 for details.";
        let qs = extract_questions(reply);
        assert_eq!(
            qs[0],
            "If you could look back from ten years in the future, what is the one thing you hope you will understand about yourself, and that no amount of advice from anyone else could have handed to you?"
        );
        assert_eq!(
            qs[1..].to_vec(),
            strings(&["Which port?", "And should I use TLS?"])
        );
    }

    #[test]
    fn inner_dots_and_urls_inside_questions() {
        assert_eq!(
            extract_questions("Is v1.2 ok? Yes."),
            strings(&["Is v1.2 ok?"])
        );
        assert_eq!(
            extract_questions("Did you read https://a.dev/x?y=1 already?"),
            strings(&["Did you read https://a.dev/x?y=1 already?"])
        );
        assert_eq!(extract_questions("Really?? ok"), strings(&["Really??"]));
        assert!(extract_questions("No questions here.").is_empty());
        let many: String = (0..30)
            .map(|i| format!("Question number {i}?\n\n"))
            .collect();
        assert_eq!(extract_questions(&many).len(), MAX_QUESTIONS);
    }

    #[test]
    fn last_assistant_text_skips_empty_and_non_assistant() {
        let entries = vec![
            json!({"type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": "first"}]}}),
            json!({"type": "message", "message": {"role": "assistant", "content": [{"type": "toolCall"}]}}),
            json!({"type": "message", "message": {"role": "user", "content": "hi"}}),
            json!({"type": "custom", "customType": "x"}),
        ];
        assert_eq!(last_assistant_text(&entries), "first");
        assert_eq!(last_assistant_text(&[]), "");
    }
}
