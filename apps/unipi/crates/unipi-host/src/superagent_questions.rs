//! Owner questions (OIN-81): a lead's own `ask_user` dialog becomes a work-pane
//! card (a `needsYou` question row) instead of popping the app dialog. The
//! ticket stays `in_progress` with its claim; the answer rides `superagent.relay`
//! back into the SAME lead session's turn (`Workers::answer`, first-wins).
//!
//! When nobody answers within `questionTimeoutMinutes` the question converts:
//! the ticket is blocked **with the claim kept** (`dispatcher::approval_transition`),
//! the lead is released from waiting and told to work its other tickets. A later
//! answer resumes the same session (`relay_to_lead` branch (2), already in place).

use oino_core::ui::UiAnswer;
use serde_json::{json, Value};

/// The timeout answer the dialog receives (so the lead's tool returns and the
/// turn ends; the question now lives on the blocked ticket).
#[must_use]
pub fn timeout_text(minutes: u32, id: &str) -> String {
    format!(
        "No answer from the owner within {minutes} min. {id} is now blocked with your question, and you still hold it. The answer will arrive later as a message in this chat. Work your other tickets meanwhile. Do not move {id} yourself."
    )
}

/// Timeout steer when an `ask_user` dialog has NO attributed `in_progress` ticket (D1).
#[must_use]
pub fn unattributed_timeout_text(minutes: u32) -> String {
    format!(
        "The owner did not answer within {minutes} min; carry on without it or put the question on a ticket."
    )
}

/// Blocked `blocked: …` texts the host plumbing produced (OIN-80 migration
/// list): legacy tickets parked by the machine, never a real owner question.
pub const PLUMBING_PREFIXES: &[&str] = &[
    "Worker was interrupted",
    "Worker ended without a transition",
    "Worker ended while blocked",
    "Worker stopped",
    "Worker could not start",
    "Lead delivery failed",
];

/// A lead's open `ask_user` dialog as the tick observed it.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadQuestion {
    /// The `UiRequest` id (`Workers::answer` targets it).
    pub request_id: String,
    /// The question texts, one per form question.
    pub texts: Vec<String>,
    /// The form's option labels per question (the work pane renders them).
    pub options: Vec<Vec<String>>,
    /// Form question ids (`q0`, `q1`, …) matching `unipi.ask-user` `form_request`.
    pub question_ids: Vec<String>,
    /// Per-question free-text policy (`other` in `unipi.ask-user`, defaults to `true`).
    pub allow_other: Vec<bool>,
}

#[derive(Debug, PartialEq)]
pub enum TickAction {
    /// Nothing open (clears a stale `record.question` too).
    None,
    /// An open dialog needs the record updated (creates or refreshes).
    Open(LeadQuestion),
    /// The dialog has been open past the timeout: convert it now.
    Timeout {
        request_id: String,
        task_id: String,
        minutes: u32,
    },
}

/// The next tick action for one lead (`None` = no change).
/// `question` is the persisted `record.question` `(request_id, first_seen_at)`.
#[must_use]
pub fn tick_action(
    questions: &[LeadQuestion],
    question: Option<&crate::superagent_leads::PendingQuestion>,
    now: i64,
    timeout_minutes: u32,
    held: &[String],
) -> TickAction {
    if let Some(pending) = question.filter(|q| q.timing_out) {
        return TickAction::Timeout {
            request_id: pending.request_id.clone(),
            task_id: pending.task_id.clone(),
            minutes: timeout_minutes,
        };
    }
    let Some(first) = questions.first() else {
        return TickAction::None;
    };
    let task_id = attributed_task(&first.texts, held)
        .or_else(|| {
            question
                .filter(|q| q.request_id == first.request_id && !q.task_id.is_empty())
                .map(|q| q.task_id.clone())
        })
        .unwrap_or_default();
    let first_seen_at = match question {
        Some(pending) if pending.request_id == first.request_id => pending.first_seen_at,
        _ => now,
    };
    if now.saturating_sub(first_seen_at) >= i64::from(timeout_minutes) * 60_000 {
        return TickAction::Timeout {
            request_id: first.request_id.clone(),
            task_id,
            minutes: timeout_minutes,
        };
    }
    TickAction::Open(first.clone())
}

/// Which held ticket this question is about (spec order): 1. the first
/// `[PREFIX]-<n>` in the question text that names a held ticket, 2. else the
/// only held `in_progress` ticket, 3. else the lowest held id.
#[must_use]
pub fn attributed_task(texts: &[String], held: &[String]) -> Option<String> {
    if held.is_empty() {
        return None;
    }
    for text in texts {
        for match_ in ticket_refs(text) {
            if held.iter().any(|id| id.eq_ignore_ascii_case(&match_)) {
                return held
                    .iter()
                    .find(|id| id.eq_ignore_ascii_case(&match_))
                    .cloned();
            }
        }
    }
    if held.len() == 1 {
        return Some(held[0].clone());
    }
    held.iter().min().cloned()
}

/// `[A-Z]+-\d+` ids in reading order, lowercased for the comparison.
fn ticket_refs(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_uppercase() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'-' {
                let dash = i;
                i += 1;
                let digits_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > digits_start && (i >= bytes.len() || !bytes[i].is_ascii_alphanumeric()) {
                    out.push(text[start..i].to_owned());
                    continue;
                }
                i = dash;
            }
        }
        i += 1;
    }
    out
}

/// The `needsYou` question row of an open question (spec shape; the excerpt is
/// "1) … 2) …" with several form questions). `askedAt` and `expiresAt` are
/// epoch-ms numbers matching the frontend contract.
#[must_use]
pub fn open_row(
    project_id: &str,
    task_id: &str,
    session_id: &str,
    question: &LeadQuestion,
    first_seen_at: i64,
    timeout_minutes: u32,
) -> Value {
    let excerpt = excerpt(&question.texts);
    json!({
        "kind": "question",
        "projectId": project_id,
        "taskId": task_id,
        "status": "in_progress",
        "question": excerpt,
        "sessionId": session_id,
        "requestId": question.request_id,
        "options": question.options,
        "askedAt": first_seen_at,
        "expiresAt": first_seen_at.saturating_add(i64::from(timeout_minutes) * 60_000),
    })
}

/// The `needsYou` question rows of every recorded open question, joined with
/// the lead status rows (project/session id). One row per record; a stale
/// record without a live dialog is skipped by the tick, and `status` trusts
/// the record (the tick clears it within a second).
pub async fn open_question_rows(
    dispatcher: &crate::superagent::Dispatcher,
    lead_rows: &[Value],
    timeout_minutes: u32,
) -> Vec<Value> {
    let mut rows = Vec::new();
    let records: Vec<(String, String, crate::superagent_leads::PendingQuestion)> = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        leads
            .leads
            .values()
            .filter_map(|record| {
                record.question.as_ref().map(|question| {
                    (
                        record.project_id.clone(),
                        record.session_id.clone(),
                        question.clone(),
                    )
                })
            })
            .collect()
    };
    for (project_id, session_id, question) in records {
        if question.task_id.is_empty() || (question.timing_out && question.board_blocked) {
            continue;
        }
        // The lead must appear in the computed lead rows (an archived project
        // disappears from status entirely).
        if !lead_rows.iter().any(|row| {
            row["projectId"] == json!(project_id) && row["sessionId"] == json!(session_id)
        }) {
            continue;
        }
        // Options come from the live dialog when present (rendered by the pane).
        let options = dispatcher
            .workers
            .questions(&session_id)
            .await
            .into_iter()
            .find(|q| q.request_id == question.request_id)
            .map_or_else(|| json!([]), |q| json!(q.options));
        let mut row = open_row(
            &project_id,
            &question.task_id,
            &session_id,
            &crate::superagent_questions::LeadQuestion {
                request_id: question.request_id,
                texts: vec![question.excerpt],
                options: Vec::new(),
                question_ids: Vec::new(),
                allow_other: Vec::new(),
            },
            question.first_seen_at,
            timeout_minutes,
        );
        row["options"] = options;
        rows.push(row);
    }
    rows
}

/// The `needsYou` question row of a blocked-with-claim ticket (the timeout
/// converted it; the answer wakes the same lead session).
#[must_use]
pub fn blocked_row(project_id: &str, task_id: &str, session_id: &str, question: &str) -> Value {
    json!({
        "kind": "question",
        "projectId": project_id,
        "taskId": task_id,
        "status": "blocked",
        "question": question,
        "sessionId": session_id,
    })
}

/// "1) … 2) …" with several form questions; single questions render bare.
#[must_use]
pub fn excerpt(texts: &[String]) -> String {
    match texts.len() {
        0 => String::new(),
        1 => texts[0].clone(),
        _ => texts
            .iter()
            .enumerate()
            .map(|(i, text)| format!("{}) {}", i + 1, text))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// True when a blocked ticket's question is host plumbing (not a lead question):
/// such rows never reach `needsYou`.
#[must_use]
pub fn is_plumbing(question: &str) -> bool {
    let trimmed = question
        .trim_start()
        .strip_prefix("blocked: ")
        .unwrap_or_else(|| question.trim_start());
    PLUMBING_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Empty form submission (`{"answers": []}`), decoded by `unipi.ask-user` as
/// `AnswerStatus::Skipped` regardless of question count or `other: false`.
#[must_use]
pub fn skipped_answer() -> UiAnswer {
    UiAnswer::Value(json!({"answers": []}))
}

/// Resolution plan for an open `ask_user` dialog: the `UiAnswer` sent to close
/// the form request plus an optional steer message delivered into the lead turn.
#[derive(Clone, Debug, PartialEq)]
pub struct EncodedAnswer {
    pub dialog_answer: UiAnswer,
    pub steer_message: Option<String>,
}

/// Encode an owner's work-pane reply against the live `ask_user` dialog:
/// - 1 question and reply matches an option label -> `selected: [label]` for that question id.
/// - 1 question and `other` is allowed -> `other: <reply>` for that question id.
/// - All other cases (`other: false` non-match, or >1 question) -> resolve the
///   dialog as skipped and steer `"Owner answered your question (\"<q>\"): <reply>"`.
#[must_use]
pub fn encode_owner_answer(
    question: Option<&LeadQuestion>,
    fallback_excerpt: &str,
    text: &str,
) -> EncodedAnswer {
    let trimmed = text.trim();
    if let Some(q) = question.filter(|q| q.texts.len() == 1) {
        let qid = q
            .question_ids
            .first()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("q0");
        let opts: &[String] = q.options.first().map_or(&[], Vec::as_slice);
        let allows_other = q.allow_other.first().copied().unwrap_or(true);
        if let Some(matched) = opts.iter().find(|opt| opt.trim() == trimmed) {
            return EncodedAnswer {
                dialog_answer: UiAnswer::Value(
                    json!({"answers": [{"id": qid, "selected": [matched]}]}),
                ),
                steer_message: None,
            };
        }
        if allows_other {
            return EncodedAnswer {
                dialog_answer: UiAnswer::Value(json!({"answers": [{"id": qid, "other": trimmed}]})),
                steer_message: None,
            };
        }
        let qtext = q.texts.first().map_or(fallback_excerpt, String::as_str);
        return EncodedAnswer {
            dialog_answer: skipped_answer(),
            steer_message: Some(format!(
                "Owner answered your question (\"{qtext}\"): {trimmed}"
            )),
        };
    }
    let qtext = question
        .map(|q| excerpt(&q.texts))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_excerpt.to_owned());
    EncodedAnswer {
        dialog_answer: skipped_answer(),
        steer_message: Some(format!(
            "Owner answered your question (\"{qtext}\"): {trimmed}"
        )),
    }
}

/// Resolution plan when a question times out: skip the dialog so `other: false`
/// or multi-question forms cannot drop the instruction, and steer the timeout
/// guidance into the lead session.
#[must_use]
pub fn timeout_resolution(minutes: u32, task_id: &str) -> EncodedAnswer {
    let message = if task_id.is_empty() {
        unattributed_timeout_text(minutes)
    } else {
        timeout_text(minutes, task_id)
    };
    EncodedAnswer {
        dialog_answer: skipped_answer(),
        steer_message: Some(message),
    }
}

/// The `superagent_status` payload (read-only): every lead's state/health plus
/// what waits for the owner, optionally limited to one project.
/// # Errors
/// Returns an error if question records or board state cannot be read.
pub async fn status_payload(
    dispatcher: &crate::superagent::Dispatcher,
    project_id: Option<&str>,
) -> Result<Value, crate::superagent::DispatchError> {
    let mut status = dispatcher.status().await?;
    if let Some(project_id) = project_id {
        let keep = |row: &Value| row["projectId"] == json!(project_id);
        for key in ["leads", "needsYou", "review", "projects"] {
            if let Some(rows) = status.get_mut(key).and_then(Value::as_array_mut) {
                rows.retain(keep);
            }
        }
    }
    let leads: Vec<Value> = status["leads"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    json!({
                        "projectId": row["projectId"], "sessionId": row["sessionId"],
                        "state": row["state"], "health": row["health"],
                        "question": row["question"], "ticketIds": row["ticketIds"],
                        "model": row["model"], "mode": row["mode"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let needs_you: Vec<Value> = status["needsYou"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    json!({
                        "kind": row.get("kind").cloned().unwrap_or_else(|| json!("approval")),
                        "projectId": row["projectId"], "taskId": row["taskId"],
                        "status": row.get("status").cloned().unwrap_or_else(|| json!("blocked")),
                        "question": row.get("question").cloned().unwrap_or_else(|| json!(row["reason"].to_string())),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let review: Vec<Value> = status["review"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| json!({"projectId": row["projectId"], "taskId": row["taskId"]}))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({"leads": leads, "needsYou": needs_you, "review": review}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(text: &str) -> LeadQuestion {
        LeadQuestion {
            request_id: "req".into(),
            texts: vec![text.into()],
            options: vec![vec!["A".into(), "B".into()]],
            question_ids: vec!["q0".into()],
            allow_other: vec![true],
        }
    }

    #[test]
    fn attribution_prefers_the_named_ticket_then_the_only_hold() {
        let held = ["T-9".to_owned(), "T-2".to_owned()];
        assert_eq!(
            attributed_task(&["T-2: which API key?".to_owned()], &held),
            Some("T-2".to_owned())
        );
        assert_eq!(
            attributed_task(&["Which API key?".to_owned()], &["T-7".to_owned()]),
            Some("T-7".to_owned())
        );
        // Not held → falls to the lowest held id.
        assert_eq!(
            attributed_task(&["T-1: really?".to_owned()], &held),
            Some("T-2".to_owned())
        );
        assert_eq!(
            attributed_task(&["Which?".to_owned()], &held),
            Some("T-2".to_owned())
        );
        assert_eq!(attributed_task(&["Which?".to_owned()], &[]), None);
        // Multi-question forms check every text.
        assert_eq!(
            attributed_task(
                &["Which?".to_owned(), "OIN-9: style?".to_owned()],
                &["OIN-9".to_owned()]
            ),
            Some("OIN-9".to_owned())
        );
    }

    #[test]
    fn ticket_refs_scan_ids_without_regex() {
        assert_eq!(
            ticket_refs("Fix OIN-12 first, see T-3 and ABC-007?"),
            ["OIN-12", "T-3", "ABC-007"]
        );
        assert_eq!(ticket_refs("no ids here 12-34"), Vec::<String>::new());
    }

    #[test]
    fn tick_action_creates_then_times_out() {
        let now = 10 * 60_000;
        let pending = crate::superagent_leads::PendingQuestion {
            request_id: "req".into(),
            task_id: "T-2".into(),
            excerpt: "Which?".into(),
            first_seen_at: now,
            timing_out: false,
            board_blocked: false,
            dialog_resolved: false,
        };
        let held = ["T-2".to_owned()];
        assert_eq!(
            tick_action(&[question("Which?")], None, now, 30, &held),
            TickAction::Open(question("Which?"))
        );
        // Same request id keeps the original first_seen_at; past the timeout → convert.
        assert_eq!(
            tick_action(
                &[question("Which?")],
                Some(&pending),
                now + 30 * 60_000,
                30,
                &held
            ),
            TickAction::Timeout {
                request_id: "req".into(),
                task_id: "T-2".into(),
                minutes: 30
            }
        );
        // A NEW request id restarts the clock (the lead asked again).
        let mut second = question("Which?");
        second.request_id = "req2".into();
        assert_eq!(
            tick_action(
                &[second.clone()],
                Some(&pending),
                now + 31 * 60_000,
                30,
                &held
            ),
            TickAction::Open(second)
        );
        // No dialog: clear.
        assert_eq!(
            tick_action(&[], Some(&pending), now, 30, &held),
            TickAction::None
        );
    }

    #[test]
    fn plumbing_texts_are_filtered_real_questions_are_not() {
        for text in PLUMBING_PREFIXES {
            assert!(is_plumbing(text), "{text}");
            assert!(is_plumbing(&format!("blocked: {text}")));
        }
        assert!(!is_plumbing("Which API key should I use?"));
        assert!(!is_plumbing("Worker decided to ask: postgres or sqlite?"));
    }

    #[test]
    fn rows_carry_the_ui_contract_fields() {
        let row = open_row("p", "T-1", "s", &question("Which API key?"), 1_000, 30);
        assert_eq!(row["kind"], "question");
        assert_eq!(row["status"], "in_progress");
        assert_eq!(row["requestId"], "req");
        assert_eq!(row["options"], json!([["A", "B"]]));
        assert_eq!(row["askedAt"], json!(1_000));
        assert_eq!(row["expiresAt"], json!(1_000 + 30 * 60_000));
        assert!(row.get("timeoutAt").is_none());
        let blocked = blocked_row("p", "T-1", "s", "Which API key?");
        assert_eq!(blocked["kind"], "question");
        assert_eq!(blocked["status"], "blocked");
        assert_eq!(blocked["sessionId"], "s");
    }

    #[test]
    fn excerpt_numbers_multi_question_forms() {
        assert_eq!(excerpt(&["one".into()]), "one");
        assert_eq!(excerpt(&["one".into(), "two".into()]), "1) one 2) two");
    }

    #[test]
    fn answers_encode_option_match_free_text_restricted_multi_and_timeout() {
        // (a) Single question + exact option match -> `selected` for that question's id.
        let mut strict = question("Which DB?");
        strict.question_ids = vec!["db_choice".into()];
        strict.options = vec![vec!["postgres".into(), "sqlite".into()]];
        strict.allow_other = vec![false];
        let matched = encode_owner_answer(Some(&strict), "Which DB?", "sqlite");
        assert_eq!(
            matched,
            EncodedAnswer {
                dialog_answer: UiAnswer::Value(
                    json!({"answers": [{"id": "db_choice", "selected": ["sqlite"]}]})
                ),
                steer_message: None,
            }
        );

        // Single question + `other: true` + non-matching text -> `other`.
        let mut free = strict.clone();
        free.allow_other = vec![true];
        let other = encode_owner_answer(Some(&free), "Which DB?", "duckdb");
        assert_eq!(
            other,
            EncodedAnswer {
                dialog_answer: UiAnswer::Value(
                    json!({"answers": [{"id": "db_choice", "other": "duckdb"}]})
                ),
                steer_message: None,
            }
        );

        // (b) Single question + `other: false` + non-matching text -> skipped + verbatim steer.
        let restricted = encode_owner_answer(Some(&strict), "Which DB?", "use cockroachdb");
        assert_eq!(
            restricted,
            EncodedAnswer {
                dialog_answer: UiAnswer::Value(json!({"answers": []})),
                steer_message: Some(
                    "Owner answered your question (\"Which DB?\"): use cockroachdb".into()
                ),
            }
        );

        // (c) Multi-question dialog -> skipped + verbatim steer quoting combined excerpt.
        let multi = LeadQuestion {
            request_id: "req-m".into(),
            texts: vec!["DB?".into(), "Region?".into()],
            options: vec![vec!["pg".into()], vec!["eu".into(), "us".into()]],
            question_ids: vec!["q0".into(), "q1".into()],
            allow_other: vec![true, true],
        };
        let encoded_multi = encode_owner_answer(Some(&multi), "1) DB? 2) Region?", "pg in eu");
        assert_eq!(
            encoded_multi,
            EncodedAnswer {
                dialog_answer: UiAnswer::Value(json!({"answers": []})),
                steer_message: Some(
                    "Owner answered your question (\"1) DB? 2) Region?\"): pg in eu".into()
                ),
            }
        );

        // (d) Timeout -> skipped + timeout steer text (attributed and unattributed).
        let timeout = timeout_resolution(30, "T-1");
        assert_eq!(
            timeout.dialog_answer,
            UiAnswer::Value(json!({"answers": []}))
        );
        let steer = timeout.steer_message.expect("timeout steer");
        assert!(steer.contains("30 min") && steer.contains("T-1"), "{steer}");

        let unattributed = timeout_resolution(30, "");
        assert_eq!(
            unattributed.dialog_answer,
            UiAnswer::Value(json!({"answers": []}))
        );
        assert_eq!(
            unattributed.steer_message.as_deref(),
            Some("The owner did not answer within 30 min; carry on without it or put the question on a ticket.")
        );
    }
}
