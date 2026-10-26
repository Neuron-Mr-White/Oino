//! Cut logic and token estimates (unipi `cut.ts`, `token-estimate.ts`, `compact-args.ts`):
//! `keep:N` user-turn cuts, the token-budget tail rescue, smart keep-tail, `""` sentinel
//! orphan recovery, chars-per-token calibration, compaction argument parsing.

use serde_json::{Value, json};

use crate::blocks::clen;

/// unipi `COMPACTOR_INSTRUCTION`.
pub const COMPACTOR_INSTRUCTION: &str = "__compactor__";
/// Default chars/token.
pub const DEFAULT_CPT: f64 = 4.0;
/// Chars attributed to one image.
pub const IMAGE_CONTENT_CHARS: usize = 4800;
/// Smart tail floor.
pub const MIN_SMART_TAIL_TOKENS: u64 = 5_000;
/// Smart tail cap.
pub const MAX_SMART_TAIL_TOKENS: u64 = 25_000;
/// Oversized tail factor.
pub const OVERSIZED_TAIL_FACTOR: f64 = 2.5;

/// Calibrated chars/token (2–6), or the default.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "char/token counts fit f64")]
pub fn calibrate(chars: usize, tokens: Option<u64>) -> f64 {
    match tokens {
        Some(t) if t > 0 && chars > 0 => {
            let raw = chars as f64 / t as f64;
            if raw.is_finite() && raw > 0.0 {
                raw.clamp(2.0, 6.0)
            } else {
                DEFAULT_CPT
            }
        }
        _ => DEFAULT_CPT,
    }
}

/// `ceil(chars / cpt)`.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positive, bounded"
)]
pub fn tokens_from_chars(chars: usize, cpt: f64) -> u64 {
    (chars as f64 / cpt).ceil() as u64
}

/// Chars of a message content (unipi `estimateMessageContentChars`).
#[must_use]
pub fn content_chars(content: Option<&Value>) -> usize {
    match content {
        Some(Value::String(s)) => clen(s),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|p| match p.get("type").and_then(Value::as_str) {
                Some("thinking") => p.get("thinking").and_then(Value::as_str).map_or(0, clen),
                Some("toolCall") => {
                    let args = p.get("arguments").or_else(|| p.get("input"));
                    let a = match args {
                        Some(Value::String(s)) => clen(s),
                        Some(v) => clen(&v.to_string()),
                        None => 2,
                    };
                    p.get("name").and_then(Value::as_str).map_or(0, clen) + a
                }
                Some("toolResult") => match p.get("content") {
                    Some(Value::String(s)) => clen(s),
                    Some(v) => clen(&v.to_string()),
                    None => 2,
                },
                Some("image") => IMAGE_CONTENT_CHARS,
                _ => p.get("text").and_then(Value::as_str).map_or(0, clen),
            })
            .sum(),
        _ => 0,
    }
}

/// Tokens of a message content.
#[must_use]
pub fn content_tokens(content: Option<&Value>, cpt: f64) -> u64 {
    tokens_from_chars(content_chars(content), cpt)
}

/// A live (context-bearing) message with its entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    /// Entry id.
    pub id: String,
    /// Message (agent form).
    pub message: Value,
}

impl Live {
    fn role(&self) -> &str {
        self.message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }
}

fn id_of(e: &Value) -> &str {
    e.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn ty(e: &Value) -> &str {
    e.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn to_live(e: &Value) -> Option<Value> {
    match ty(e) {
        "message" => e.get("message").cloned(),
        "custom_message" => Some(json!({
            "role": "custom", "customType": e.get("customType"), "content": e.get("content"),
            "display": e.get("display"), "details": e.get("details"),
        })),
        "branch_summary" => Some(json!({
            "role": "branchSummary", "summary": e.get("summary"), "fromId": e.get("fromId"),
        })),
        _ => None,
    }
}

/// Live messages after the last compaction's kept entry (orphan recovery on `""`/missing).
#[must_use]
pub fn collect_live(branch: &[Value]) -> Vec<Live> {
    let last = branch.iter().rposition(|e| ty(e) == "compaction");
    let kept = last.and_then(|i| {
        branch[i]
            .get("firstKeptEntryId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    let valid = kept
        .as_deref()
        .is_some_and(|k| !k.is_empty() && branch.iter().any(|e| id_of(e) == k));
    let mut out = Vec::new();
    let push = |e: &Value, out: &mut Vec<Live>| {
        if ty(e) == "compaction" {
            return;
        }
        if let Some(m) = to_live(e) {
            out.push(Live {
                id: id_of(e).to_owned(),
                message: m,
            });
        }
    };
    if let (Some(li), false) = (last, valid) {
        for e in &branch[li + 1..] {
            push(e, &mut out);
        }
    } else {
        let mut found = kept.is_none();
        for e in branch {
            if !found && Some(id_of(e)) == kept.as_deref() {
                found = true;
            }
            if found {
                push(e, &mut out);
            }
        }
    }
    out
}

/// Why a cut was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No live messages.
    NoLiveMessages,
    /// ≤ 2 live messages.
    TooFewLiveMessages,
}

impl Refusal {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoLiveMessages => "no_live_messages",
            Self::TooFewLiveMessages => "too_few_live_messages",
        }
    }
}

/// How a budget cut was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetCut {
    /// No user turn to anchor on.
    NoAnchor,
    /// The user-turn tail was oversized.
    OversizedTail,
}

impl BudgetCut {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoAnchor => "no_anchor",
            Self::OversizedTail => "oversized_tail",
        }
    }
}

/// A cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// Messages summarized.
    pub messages: Vec<Value>,
    /// First kept entry (`""` = compact all).
    pub first_kept: String,
    /// Everything compacted.
    pub compact_all: bool,
    /// Kept user turns.
    pub kept_user_turns: usize,
    /// Total user turns.
    pub total_user_turns: usize,
    /// Requested keep.
    pub requested_keep: usize,
    /// Keep could not form a boundary.
    pub keep_fallback: bool,
    /// Budget cut kind.
    pub budget_cut: Option<BudgetCut>,
}

/// unipi `buildOwnCut`.
///
/// # Errors
///
/// [`Refusal`] when there is nothing (or too little) to compact.
pub fn build_own_cut(branch: &[Value], keep: usize) -> Result<Cut, Refusal> {
    let live = collect_live(branch);
    if live.is_empty() {
        return Err(Refusal::NoLiveMessages);
    }
    if live.len() <= 2 {
        return Err(Refusal::TooFewLiveMessages);
    }
    let users: Vec<usize> = live
        .iter()
        .enumerate()
        .filter(|(_, l)| l.role() == "user")
        .map(|(i, _)| i)
        .collect();
    let all = |fallback: bool| Cut {
        messages: live.iter().map(|l| l.message.clone()).collect(),
        first_kept: String::new(),
        compact_all: true,
        kept_user_turns: 0,
        total_user_turns: users.len(),
        requested_keep: keep,
        keep_fallback: fallback,
        budget_cut: None,
    };
    if keep == 0 {
        return Ok(all(false));
    }
    let cut_idx = users.len().checked_sub(keep).map_or(0, |t| users[t]);
    if cut_idx == 0 {
        return Ok(all(true));
    }
    Ok(Cut {
        messages: live[..cut_idx].iter().map(|l| l.message.clone()).collect(),
        first_kept: live[cut_idx].id.clone(),
        compact_all: false,
        kept_user_turns: keep,
        total_user_turns: users.len(),
        requested_keep: keep,
        keep_fallback: false,
        budget_cut: None,
    })
}

/// Index where the newest `max_tokens` start (snapped off tool results), or `None`.
#[must_use]
pub fn find_budget_cut(live: &[Live], max_tokens: u64, cpt: f64) -> Option<usize> {
    let mut acc = 0;
    let mut crossed = None;
    for i in (0..live.len()).rev() {
        acc += content_tokens(live[i].message.get("content"), cpt);
        if acc >= max_tokens {
            crossed = Some(i);
            break;
        }
    }
    let c = crossed?;
    (c.max(1)..live.len()).find(|j| live[*j].role() != "toolResult")
}

/// unipi `applyTailBudget`.
#[must_use]
pub fn apply_tail_budget(
    branch: &[Value],
    cut: Cut,
    max_tokens: Option<u64>,
    factor: Option<f64>,
    cpt: f64,
) -> Cut {
    let max = max_tokens.unwrap_or(MAX_SMART_TAIL_TOKENS);
    let factor = factor.unwrap_or(OVERSIZED_TAIL_FACTOR);
    let live = collect_live(branch);
    let budget = |idx: usize, kind: BudgetCut| Cut {
        messages: live[..idx].iter().map(|l| l.message.clone()).collect(),
        first_kept: live[idx].id.clone(),
        compact_all: false,
        kept_user_turns: live[idx..].iter().filter(|l| l.role() == "user").count(),
        total_user_turns: live.iter().filter(|l| l.role() == "user").count(),
        requested_keep: cut.requested_keep,
        keep_fallback: false,
        budget_cut: Some(kind),
    };
    if cut.compact_all {
        if !cut.keep_fallback {
            return cut;
        }
        return match find_budget_cut(&live, max, cpt) {
            Some(i) => budget(i, BudgetCut::NoAnchor),
            None => cut,
        };
    }
    let start = cut.messages.len();
    let tail: u64 = live
        .iter()
        .skip(start)
        .map(|l| content_tokens(l.message.get("content"), cpt))
        .sum();
    #[allow(clippy::cast_precision_loss, reason = "token counts fit f64")]
    if tail as f64 <= max as f64 * factor {
        return cut;
    }
    match find_budget_cut(&live, max, cpt) {
        Some(i) if i > start => budget(i, BudgetCut::OversizedTail),
        _ => cut,
    }
}

fn tail_tokens_for_keep(branch: &[Value], keep: usize, cpt: f64) -> Option<u64> {
    let cut = build_own_cut(branch, keep).ok()?;
    if cut.compact_all {
        return None;
    }
    let idx = branch.iter().position(|e| id_of(e) == cut.first_kept)?;
    let chars: usize = branch[idx..]
        .iter()
        .filter(|e| ty(e) == "message")
        .map(|e| content_chars(e.get("message").and_then(|m| m.get("content"))))
        .sum();
    Some(tokens_from_chars(chars, cpt))
}

/// Smart keep result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartKeep {
    /// Effective keep.
    pub keep: usize,
    /// Boosted beyond the base.
    pub adjusted: bool,
    /// Base keep.
    pub from: usize,
}

/// unipi `resolveSmartKeepUserTurns`.
#[must_use]
pub fn resolve_smart_keep(
    branch: &[Value],
    requested: Option<usize>,
    explicit: bool,
    smart: bool,
    cpt: f64,
) -> SmartKeep {
    let base = requested.unwrap_or(1);
    let unchanged = SmartKeep {
        keep: base,
        adjusted: false,
        from: base,
    };
    if explicit || !smart {
        return unchanged;
    }
    match tail_tokens_for_keep(branch, base, cpt) {
        Some(t) if t <= MIN_SMART_TAIL_TOKENS => {}
        _ => return unchanged,
    }
    let total = build_own_cut(branch, base).map_or(0, |c| c.total_user_turns);
    let mut selected = base;
    for k in base + 1..=total {
        match tail_tokens_for_keep(branch, k, cpt) {
            Some(t) if t <= MAX_SMART_TAIL_TOKENS => selected = k,
            _ => break,
        }
    }
    SmartKeep {
        keep: selected,
        adjusted: selected != base,
        from: base,
    }
}

/// Parsed compaction instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instructions {
    /// Ours (the marker).
    pub is_compactor: bool,
    /// keep:N.
    pub keep: usize,
    /// keep:N given.
    pub explicit: bool,
    /// Free text to send after compaction.
    pub follow_up: Option<String>,
}

fn parse_keep_number(s: &str) -> usize {
    s.parse::<usize>().unwrap_or(usize::MAX)
}

/// unipi `parseKeepAndPrompt` → `(prompt, keep)`.
#[must_use]
pub fn parse_keep_and_prompt(args: &str) -> (String, Option<usize>) {
    let t = args.trim();
    if t.is_empty() {
        return (String::new(), None);
    }
    if let Some(rest) = t.strip_prefix("keep:") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let after = &rest[digits.len()..];
        if !digits.is_empty() && (after.is_empty() || after.starts_with(char::is_whitespace)) {
            return (after.trim().to_owned(), Some(parse_keep_number(&digits)));
        }
    }
    let last = t.split_whitespace().last().unwrap_or_default();
    if let Some(d) = last.strip_prefix("keep:")
        && !d.is_empty()
        && d.chars().all(|c| c.is_ascii_digit())
    {
        return (
            t[..t.len() - last.len()].trim().to_owned(),
            Some(parse_keep_number(d)),
        );
    }
    (t.to_owned(), None)
}

/// unipi `parseCompactionInstructions`.
#[must_use]
pub fn parse_instructions(custom: Option<&str>) -> Instructions {
    let trimmed = custom.map(str::trim);
    if trimmed == Some(COMPACTOR_INSTRUCTION) {
        return Instructions {
            is_compactor: true,
            keep: 1,
            explicit: false,
            follow_up: None,
        };
    }
    if let Some(rest) = trimmed.and_then(|t| t.strip_prefix(COMPACTOR_INSTRUCTION))
        && rest.starts_with(char::is_whitespace)
    {
        let first = rest.trim().lines().next().unwrap_or_default();
        let (_, keep) = parse_keep_and_prompt(first);
        return Instructions {
            is_compactor: true,
            keep: keep.unwrap_or(1),
            explicit: keep.is_some(),
            follow_up: None,
        };
    }
    let (prompt, keep) = parse_keep_and_prompt(custom.unwrap_or_default());
    Instructions {
        is_compactor: false,
        keep: keep.unwrap_or(1),
        explicit: keep.is_some(),
        follow_up: (!prompt.is_empty()).then_some(prompt),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Session-branch builders shared by the tests (unipi `tests/fixtures.ts`).
    use std::cell::Cell;

    use serde_json::{Value, json};

    thread_local!(static SEQ: Cell<u32> = const { Cell::new(0) });
    fn id() -> String {
        SEQ.with(|s| {
            s.set(s.get() + 1);
            format!("e{}", s.get())
        })
    }
    pub(crate) fn user(text: &str) -> Value {
        json!({"id": id(), "type": "message", "message": {"role": "user", "content": text}})
    }
    pub(crate) fn assistant(text: &str, calls: &[(&str, Value)]) -> Value {
        let mut content: Vec<Value> = Vec::new();
        if !text.is_empty() {
            content.push(json!({"type": "text", "text": text}));
        }
        for (i, (n, a)) in calls.iter().enumerate() {
            content.push(
                json!({"type": "toolCall", "id": format!("tc{i}"), "name": n, "arguments": a}),
            );
        }
        json!({"id": id(), "type": "message", "message": {"role": "assistant", "content": content}})
    }
    pub(crate) fn tool_result(name: &str, text: &str, err: bool) -> Value {
        json!({"id": id(), "type": "message", "message": {"role": "toolResult", "toolName": name, "toolCallId": "r", "content": [{"type": "text", "text": text}], "isError": err}})
    }
    pub(crate) fn custom(t: &str, content: &str) -> Value {
        json!({"id": id(), "type": "custom_message", "customType": t, "content": content, "display": false})
    }
    pub(crate) fn origin_mark(key: &str) -> Value {
        json!({"id": id(), "type": "custom", "customType": "compactor-origin", "data": {"key": key}})
    }
    pub(crate) fn compaction(summary: &str, kept: &str) -> Value {
        json!({"id": id(), "type": "compaction", "summary": summary, "firstKeptEntryId": kept, "tokensBefore": 100_000})
    }
    pub(crate) fn working_session() -> Vec<Value> {
        vec![
            user("Build a login page. Keep the existing orange theme, don't add new dependencies."),
            assistant("I'll start with the form.", &[("edit", json!({"path": "/repo/src/Login.tsx", "oldText": "a", "newText": "b"}))]),
            tool_result("edit", "ok", false),
            assistant("", &[("bash", json!({"command": "npm test"}))]),
            tool_result("bash", "1 failing: login rejects empty password", true),
            assistant("", &[("bash", json!({"command": "npm test"}))]),
            tool_result("bash", "all tests pass", false),
            assistant("", &[("bash", json!({"command": "git commit -am \"feat: login page\""}))]),
            tool_result("bash", "[main abc1234] feat: login page\n 1 file changed, 10 insertions(+)", false),
            assistant(&"The login page is done: the form validates input, the tests pass, and the change is committed as abc1234. It uses the existing orange theme and adds no dependencies. Next I would add a password reset link, which needs a decision on whether reset emails come from the existing mailer or a new service. ".repeat(3), &[]),
            user("Continue"),
            user("Now add a password reset link."),
            assistant("Adding the reset link next.", &[("read", json!({"path": "/repo/src/Login.tsx"}))]),
            tool_result("read", "export function Login() {}", false),
        ]
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn tokens() {
        assert!((calibrate(0, Some(10)) - 4.0).abs() < f64::EPSILON);
        assert!((calibrate(100, Some(10)) - 6.0).abs() < f64::EPSILON);
        assert!((calibrate(30, Some(10)) - 3.0).abs() < f64::EPSILON);
        assert_eq!(tokens_from_chars(9, 4.0), 3);
        let c = json!([{"type": "text", "text": "abcd"}, {"type": "image"}, {"type": "toolCall", "name": "ab", "arguments": {"x": 1}}]);
        assert_eq!(content_chars(Some(&c)), 4 + 4800 + 2 + 7);
    }

    #[test]
    fn cuts() {
        let b = working_session();
        let c = build_own_cut(&b, 1).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(c.first_kept, b[11]["id"].as_str().unwrap_or_default());
        assert_eq!((c.kept_user_turns, c.total_user_turns), (1, 3));
        let all = build_own_cut(&b, 0).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(all.compact_all && !all.keep_fallback);
        let big = build_own_cut(&b, 9).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(big.compact_all && big.keep_fallback);
        assert_eq!(build_own_cut(&[], 1), Err(Refusal::NoLiveMessages));
        assert_eq!(
            build_own_cut(&[user("a"), user("b")], 1),
            Err(Refusal::TooFewLiveMessages)
        );
        // Orphan recovery: "" sentinel collects after the compaction.
        let mut b2 = b.clone();
        b2.push(compaction("s", ""));
        b2.extend([user("x"), assistant("y", &[]), user("z")]);
        assert_eq!(collect_live(&b2).len(), 3);
        let mut b3 = b.clone();
        let kept = b[11]["id"].as_str().unwrap_or_default().to_owned();
        b3.push(compaction("s", &kept));
        assert_eq!(collect_live(&b3).len(), 3);
    }

    #[test]
    fn smart_keep_and_budget() {
        let b = working_session();
        let s = resolve_smart_keep(&b, None, false, true, 4.0);
        assert!(s.adjusted && s.keep > 1);
        assert_eq!(resolve_smart_keep(&b, Some(1), true, true, 4.0).keep, 1);
        let mut auto = vec![user("start")];
        for i in 0..30 {
            auto.push(assistant(
                &"x".repeat(8000),
                &[("bash", json!({"command": format!("echo {i}")}))],
            ));
            auto.push(tool_result("bash", "ok", false));
        }
        let cut = build_own_cut(&auto, 1).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(cut.compact_all && cut.keep_fallback);
        let budget = apply_tail_budget(&auto, cut, None, None, 4.0);
        assert_eq!(budget.budget_cut, Some(BudgetCut::NoAnchor));
        assert!(!budget.first_kept.is_empty());
    }

    #[test]
    fn instructions() {
        assert_eq!(
            parse_keep_and_prompt("keep:3 do it"),
            ("do it".into(), Some(3))
        );
        assert_eq!(
            parse_keep_and_prompt("do it keep:2"),
            ("do it".into(), Some(2))
        );
        assert_eq!(
            parse_keep_and_prompt("just text"),
            ("just text".into(), None)
        );
        assert_eq!(parse_keep_and_prompt("keep:x"), ("keep:x".into(), None));
        let p = parse_instructions(Some("__compactor__"));
        assert!(p.is_compactor && p.keep == 1 && !p.explicit);
        let p = parse_instructions(Some("__compactor__ keep:4\nmore"));
        assert!(p.is_compactor && p.keep == 4 && p.explicit && p.follow_up.is_none());
        let p = parse_instructions(Some("focus on the login bug"));
        assert!(!p.is_compactor);
        assert_eq!(p.follow_up.as_deref(), Some("focus on the login bug"));
        assert!(!parse_instructions(Some("__compactor__x")).is_compactor);
    }
}
