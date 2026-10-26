//! Skill exposure judging (unipi `src/judge.ts`): pins, jev scoring, the frozen per-session
//! set, the hidden-skills index and later reveals.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unipi_kit::jev::Answers;

use crate::settings::Exposure;

/// Freeze record entry.
pub const SKILLS_JUDGED_ENTRY: &str = "unipi:skills-judged";
/// Reveal record entry.
pub const SKILLS_REVEALED_ENTRY: &str = "unipi:skills-revealed";
/// System-prompt section naming the hidden skills.
pub const HIDDEN_SECTION: &str = "unipi-skills-hidden";
/// Reveal message type.
pub const REVEALED_MESSAGE: &str = "unipi-skills-revealed";
/// At most this many skills announced per later prompt.
pub const REVEAL_CAP: usize = 5;

/// One catalog entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Skill directory.
    pub location: String,
}

const COMMON: &[&str] = &[
    "work", "create", "list", "show", "find", "help", "test", "tests", "mode", "auto", "quick",
    "review", "execute", "gather", "context", "scan", "issues", "full", "output", "write",
    "writing", "agent", "agents", "user", "with", "your", "from", "this", "that", "into", "gain",
    "debt", "teach", "plan", "check", "tool", "tools", "file", "files", "code", "docs", "task",
    "tasks", "make", "build",
];

fn is_common(w: &str) -> bool {
    COMMON.contains(&w)
}

fn words(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|w| !w.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Name tokens of ≥ 4 chars.
#[must_use]
pub fn name_tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|t| t.chars().count() >= 4)
        .map(ToOwned::to_owned)
        .collect()
}

/// Removes pi's `[Image #N]` placeholders.
fn strip_image_placeholders(prompt: &str) -> String {
    let mut out = String::new();
    let mut rest = prompt;
    loop {
        let lower = rest.to_lowercase();
        let Some(i) = lower.find("[image #") else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[i + 8..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(']') {
            out.push_str(&rest[..i]);
            out.push(' ');
            rest = &after[digits + 1..];
        } else {
            out.push_str(&rest[..i + 8]);
            rest = after;
        }
    }
}

fn bounded_mention(hay: &str, needle: &str) -> bool {
    let ok = |c: Option<char>| {
        c.is_none_or(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    };
    hay.match_indices(needle).any(|(i, _)| {
        i > 0
            && ok(hay[..i].chars().next_back())
            && hay[i + needle.len()..].chars().next().is_some()
            && ok(hay[i + needle.len()..].chars().next())
    })
}

/// Skills the prompt names.
#[must_use]
pub fn pinned_skills(entries: &[Entry], prompt: &str) -> BTreeSet<String> {
    let clean = strip_image_placeholders(prompt);
    let lower = format!(
        " {} ",
        clean
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
    let prompt_words = words(&clean);
    let mut freq: HashMap<String, usize> = HashMap::new();
    for e in entries {
        for t in name_tokens(&e.name).into_iter().collect::<BTreeSet<_>>() {
            *freq.entry(t).or_default() += 1;
        }
    }
    let mut pins = BTreeSet::new();
    for e in entries {
        let n = e.name.to_lowercase();
        if lower.contains(&format!("/skill:{n}")) {
            pins.insert(e.name.clone());
            continue;
        }
        let nameable = n.contains('-') || !is_common(&n);
        if nameable
            && (lower.contains(&format!(" {} ", n.replace('-', " ")))
                || bounded_mention(&lower, &n))
        {
            pins.insert(e.name.clone());
            continue;
        }
        if name_tokens(&e.name)
            .iter()
            .any(|t| !is_common(t) && freq.get(t) == Some(&1) && prompt_words.contains(t))
        {
            pins.insert(e.name.clone());
        }
    }
    pins
}

/// unipi `isChatter`: greetings, thanks, acknowledgements, bare nudges, slash commands.
#[must_use]
pub fn is_chatter(text: &str) -> bool {
    let t = text.trim();
    if t.chars().count() < 3 || t.starts_with('/') {
        return true;
    }
    let core = t
        .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '!' | '?' | ','))
        .to_lowercase();
    let mut forms: Vec<String> = Vec::new();
    for g in ["hi", "hello", "hey", "howdy"] {
        forms.push(g.into());
        forms.push(format!("{g} there"));
    }
    for t in ["thank", "thanks"] {
        for you in ["", " you"] {
            for much in ["", " so much"] {
                forms.push(format!("{t}{you}{much}"));
            }
        }
    }
    forms.extend(
        [
            "yo",
            "sup",
            "good morning",
            "good afternoon",
            "good evening",
            "thx",
            "ty",
            "cheers",
            "nice",
            "great",
            "cool",
            "perfect",
            "awesome",
            "ok",
            "okay",
            "k",
            "yes",
            "yep",
            "yeah",
            "no",
            "nope",
            "sure",
            "lgtm",
            "sounds good",
            "go",
            "go on",
            "go ahead",
            "continue",
            "proceed",
            "retry",
            "again",
            "do it",
            "next",
            "done",
        ]
        .map(ToOwned::to_owned),
    );
    forms.contains(&core)
}

/// The jev request scoring each entry.
#[must_use]
pub fn judge_request(entries: &[Entry], prompt: &str, cwd_base: &str) -> (String, Value) {
    let state = format!(
        "{}\n[project: {cwd_base}]",
        prompt.chars().take(4000).collect::<String>()
    );
    let mut q = serde_json::Map::new();
    for (i, e) in entries.iter().enumerate() {
        q.insert(
            format!("s{i}"),
            json!({"type": "noul", "instructions": format!(
                "Would the skill \"{}\" help handle this request? Skill: {}",
                e.name,
                e.description.chars().take(300).collect::<String>()
            )}),
        );
    }
    (state, Value::Object(q))
}

fn score(answers: Option<&Answers>, i: usize) -> f64 {
    answers
        .and_then(|a| a.get(&format!("s{i}")))
        .and_then(|a| a.noul)
        .unwrap_or(0.0)
}

/// Pins stay; others need ≥ threshold, best first, total capped at `maxSkills`.
#[must_use]
pub fn apply_judgement(
    entries: &[Entry],
    answers: Option<&Answers>,
    threshold: f64,
    max_skills: usize,
    pins: &BTreeSet<String>,
) -> (Vec<Entry>, Vec<Entry>) {
    let mut keep: BTreeSet<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| pins.contains(&e.name))
        .map(|(i, _)| i)
        .collect();
    let mut scored: Vec<(usize, f64)> = (0..entries.len())
        .filter(|i| !keep.contains(i))
        .map(|i| (i, score(answers, i)))
        .filter(|(_, s)| *s >= threshold)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, _) in scored {
        if keep.len() >= max_skills.max(1) {
            break;
        }
        keep.insert(i);
    }
    let (mut kept, mut hidden) = (Vec::new(), Vec::new());
    for (i, e) in entries.iter().enumerate() {
        if keep.contains(&i) {
            kept.push(e.clone());
        } else {
            hidden.push(e.clone());
        }
    }
    (kept, hidden)
}

/// `~`-shortens a path under `home`.
#[must_use]
pub fn tilde(path: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() && path.starts_with(h) => format!("~{}", &path[h.len()..]),
        _ => path.to_owned(),
    }
}

/// Static section naming every hidden skill grouped by folder.
#[must_use]
pub fn hidden_index(hidden: &[Entry], home: Option<&str>) -> String {
    if hidden.is_empty() {
        return String::new();
    }
    let mut sorted = hidden.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut odd = Vec::new();
    for e in &sorted {
        let (parent, base) = e
            .location
            .rsplit_once('/')
            .map_or(("", e.location.as_str()), |(p, b)| (p, b));
        if !e.location.is_empty() && base == e.name {
            let parent = tilde(if parent.is_empty() { "/" } else { parent }, home);
            match groups.iter_mut().find(|(p, _)| *p == parent) {
                Some((_, names)) => names.push(e.name.clone()),
                None => groups.push((parent, vec![e.name.clone()])),
            }
        } else {
            odd.push(format!(
                "{} ({}/SKILL.md)",
                e.name,
                tilde(&e.location, home)
            ));
        }
    }
    let mut lines = vec!["Other installed skills — not listed above for this session, but available. To use one, read <folder>/<name>/SKILL.md, or tell the user it can be run as /skill:<name>.".to_owned()];
    for (folder, names) in groups {
        lines.push(format!("- {folder}/: {}", names.join(", ")));
    }
    for o in odd {
        lines.push(format!("- {o}"));
    }
    lines.join("\n")
}

/// Per-session frozen state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    /// Judged and frozen.
    pub judged: bool,
    /// Kept names.
    pub kept: Vec<String>,
    /// Hidden entries.
    pub hidden: Vec<Entry>,
    /// Frozen index text.
    pub index: String,
    /// Already revealed.
    pub revealed: BTreeSet<String>,
}

/// Rebuilds the frozen state from session entries.
#[must_use]
pub fn restore_state(entries: &[Value]) -> SessionState {
    let mut s = SessionState::default();
    for e in entries {
        let data = e.get("data");
        match e.get("customType").and_then(Value::as_str) {
            Some(SKILLS_JUDGED_ENTRY) => {
                let (Some(kept), Some(hidden)) = (
                    data.and_then(|d| d.get("kept")).and_then(Value::as_array),
                    data.and_then(|d| d.get("hidden")).and_then(Value::as_array),
                ) else {
                    continue;
                };
                s.judged = true;
                s.kept = kept
                    .iter()
                    .map(|k| k.as_str().map_or_else(|| k.to_string(), ToOwned::to_owned))
                    .collect();
                let field = |h: &Value, k: &str| {
                    h.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                s.hidden = hidden
                    .iter()
                    .map(|h| Entry {
                        name: field(h, "name"),
                        description: field(h, "description"),
                        location: field(h, "location"),
                    })
                    .collect();
                data.and_then(|d| d.get("index"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .clone_into(&mut s.index);
                s.revealed.clear();
            }
            Some(SKILLS_REVEALED_ENTRY) => {
                for n in data
                    .and_then(|d| d.get("names"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    s.revealed.insert(n.as_str().unwrap_or_default().to_owned());
                }
            }
            _ => {}
        }
    }
    s
}

/// One turn's decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnOutput {
    /// Names listed this turn.
    pub listed: BTreeSet<String>,
    /// Hidden section text (`""` = none).
    pub index: String,
    /// Skills to announce.
    pub reveal: Vec<Entry>,
    /// Freeze record to persist.
    pub freeze: Option<Value>,
    /// Status text.
    pub status: Option<String>,
}

/// Inputs of a turn.
pub struct TurnInput<'a> {
    /// Prompt text.
    pub prompt: &'a str,
    /// Candidates.
    pub catalog: &'a [Entry],
    /// Exposure settings.
    pub settings: &'a Exposure,
    /// cwd basename.
    pub cwd_base: &'a str,
    /// Must-show names.
    pub must_show: &'a BTreeSet<String>,
    /// Home for `~`.
    pub home: Option<&'a str>,
}

#[allow(
    clippy::too_many_lines,
    reason = "first turn and frozen turns, as in unipi"
)]
/// unipi `decideTurn`; `ask(state, questions)` is the jev transport (`None` = unavailable).
pub async fn decide_turn<F, Fut>(
    input: &TurnInput<'_>,
    state: &mut SessionState,
    mut ask: F,
) -> TurnOutput
where
    F: FnMut(String, Value) -> Fut,
    Fut: Future<Output = Option<Answers>>,
{
    let all: BTreeSet<String> = input.catalog.iter().map(|e| e.name.clone()).collect();
    let max = input.settings.max_skills;
    if !state.judged {
        if input.catalog.len() <= max {
            return TurnOutput {
                listed: all,
                ..TurnOutput::default()
            };
        }
        if is_chatter(input.prompt) {
            return TurnOutput {
                listed: all,
                status: Some("skills: waiting for a real request".into()),
                ..TurnOutput::default()
            };
        }
        let mut pins = pinned_skills(input.catalog, input.prompt);
        pins.extend(input.must_show.iter().cloned());
        let (s, q) = judge_request(input.catalog, input.prompt, input.cwd_base);
        let answers = ask(s, q).await;
        let fail_open = answers.is_none();
        let (kept, hidden) = if fail_open {
            (input.catalog.to_vec(), Vec::new())
        } else {
            apply_judgement(
                input.catalog,
                answers.as_ref(),
                input.settings.threshold,
                max,
                &pins,
            )
        };
        let index = hidden_index(&hidden, input.home);
        state.judged = true;
        state.kept = kept.iter().map(|e| e.name.clone()).collect();
        state.hidden.clone_from(&hidden);
        state.index.clone_from(&index);
        let mut freeze = json!({"kept": state.kept, "hidden": hidden, "index": index});
        if fail_open {
            freeze["failOpen"] = json!(true);
        }
        return TurnOutput {
            listed: state.kept.iter().cloned().collect(),
            index,
            reveal: Vec::new(),
            freeze: Some(freeze),
            status: Some(format!(
                "skills: {}/{} listed",
                kept.len(),
                input.catalog.len()
            )),
        };
    }
    let mut listed: BTreeSet<String> = state
        .kept
        .iter()
        .filter(|n| all.contains(*n))
        .cloned()
        .collect();
    for n in input.must_show {
        if all.contains(n) {
            listed.insert(n.clone());
        }
    }
    let candidates: Vec<Entry> = state
        .hidden
        .iter()
        .filter(|h| {
            all.contains(&h.name) && !state.revealed.contains(&h.name) && !listed.contains(&h.name)
        })
        .cloned()
        .collect();
    let mut reveal = Vec::new();
    if !candidates.is_empty() && !is_chatter(input.prompt) {
        let pins = pinned_skills(&candidates, input.prompt);
        reveal.extend(
            candidates
                .iter()
                .filter(|c| pins.contains(&c.name))
                .cloned(),
        );
        if input.settings.recheck && reveal.len() < REVEAL_CAP {
            let rest: Vec<Entry> = candidates
                .iter()
                .filter(|c| !pins.contains(&c.name))
                .cloned()
                .collect();
            if !rest.is_empty() {
                let (s, q) = judge_request(&rest, input.prompt, input.cwd_base);
                if let Some(answers) = ask(s, q).await {
                    for (i, c) in rest.iter().enumerate() {
                        if answers
                            .get(&format!("s{i}"))
                            .and_then(|a| a.noul)
                            .is_some_and(|sc| sc >= input.settings.threshold)
                        {
                            reveal.push(c.clone());
                        }
                    }
                }
            }
        }
    }
    reveal.truncate(REVEAL_CAP);
    for r in &reveal {
        state.revealed.insert(r.name.clone());
    }
    TurnOutput {
        status: Some(format!(
            "skills: {}/{} listed",
            listed.len(),
            listed.len() + state.hidden.len()
        )),
        listed,
        index: state.index.clone(),
        reveal,
        freeze: None,
    }
}

/// The reveal message text.
#[must_use]
pub fn reveal_message(reveal: &[Entry], home: Option<&str>) -> String {
    let lines: Vec<String> = reveal
        .iter()
        .map(|r| {
            format!(
                "- {} — {} ({}/SKILL.md)",
                r.name,
                r.description.chars().take(200).collect::<String>(),
                tilde(&r.location, home)
            )
        })
        .collect();
    format!(
        "Relevant skills for this request (not in your skills list):\n{}\nRead a skill's SKILL.md before using it.",
        lines.join("\n")
    )
}

/// Answers map helper (tests and fakes).
#[must_use]
pub fn answers_from(scores: &BTreeMap<String, f64>) -> Answers {
    scores
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                unipi_kit::jev::Answer {
                    noul: Some(*v),
                    ..Default::default()
                },
            )
        })
        .collect()
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
    use std::task::{Context, Poll, Waker};

    use super::*;

    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("future not ready"),
        }
    }

    fn e(name: &str) -> Entry {
        Entry {
            name: name.into(),
            description: format!("{name} skill"),
            location: format!("/home/u/.agents/skills/{name}"),
        }
    }

    fn catalog() -> Vec<Entry> {
        [
            "coffee-sandbox",
            "unipi-v3-development",
            "agent-browser",
            "browser-automation",
            "mempalace",
            "mempalace-recall",
            "mempalace-task",
            "grill-me",
            "work",
            "plan",
            "research",
            "gather-context",
            "quick-work",
            "brainstorm",
            "debug",
        ]
        .into_iter()
        .map(e)
        .collect()
    }

    /// jev fake scoring by skill name from the question text.
    fn scorer(
        scores: &[(&str, f64)],
    ) -> impl FnMut(String, Value) -> std::future::Ready<Option<Answers>> {
        let scores: BTreeMap<String, f64> =
            scores.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
        move |_s, q: Value| {
            let mut m = BTreeMap::new();
            for (k, v) in q.as_object().into_iter().flatten() {
                let instr = v["instructions"].as_str().unwrap_or_default();
                let name = instr.split('"').nth(1).unwrap_or_default().to_owned();
                m.insert(k.clone(), scores.get(&name).copied().unwrap_or(0.0));
            }
            std::future::ready(Some(answers_from(&m)))
        }
    }

    fn none(_: String, _: Value) -> std::future::Ready<Option<Answers>> {
        std::future::ready(None)
    }

    fn exposure(max: usize, recheck: bool) -> Exposure {
        Exposure {
            max_skills: max,
            recheck,
            ..Exposure::default()
        }
    }

    #[test]
    fn pins() {
        let c = catalog();
        assert_eq!(
            pinned_skills(&c, "ssh into coffee and run pi there")
                .into_iter()
                .collect::<Vec<_>>(),
            ["coffee-sandbox"]
        );
        assert!(
            pinned_skills(&c, "use agent-browser to open example.com").contains("agent-browser")
        );
        assert!(pinned_skills(&c, "grill me about this design").contains("grill-me"));
        assert!(pinned_skills(&c, "/skill:plan the migration").contains("plan"));
        let p = pinned_skills(&c, "please work on the plan and review the browser code");
        assert!(!p.contains("work") && !p.contains("plan") && !p.contains("agent-browser"));
        assert!(pinned_skills(&c, "search mempalace").contains("mempalace"));
        assert!(!pinned_skills(&c, "search mempalace").contains("mempalace-task"));
        let mut with_image = c;
        with_image.push(e("image"));
        assert!(
            !pinned_skills(
                &with_image,
                "[Image #1]\n\nLook at this. [Image #2] [Image #3]"
            )
            .contains("image")
        );
        assert!(pinned_skills(&with_image, "generate an image of a cup").contains("image"));
    }

    #[test]
    fn judgement_and_index() {
        let c = catalog();
        let scores = |m: &[(&str, f64)]| {
            let map: BTreeMap<String, f64> = c
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    (
                        format!("s{i}"),
                        m.iter().find(|(n, _)| *n == e.name).map_or(0.0, |x| x.1),
                    )
                })
                .collect();
            answers_from(&map)
        };
        let (kept, _) = apply_judgement(
            &c,
            Some(&scores(&[("work", 0.9)])),
            0.3,
            12,
            &BTreeSet::from(["coffee-sandbox".to_owned()]),
        );
        assert_eq!(
            kept.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
            ["coffee-sandbox", "work"]
        );
        let (kept, hidden) = apply_judgement(
            &c,
            Some(&scores(&[
                ("agent-browser", 0.95),
                ("browser-automation", 0.8),
                ("mempalace", 0.2),
            ])),
            0.3,
            1,
            &BTreeSet::new(),
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "agent-browser");
        assert_eq!(hidden.len(), c.len() - 1);
        let text = hidden_index(
            &[
                Entry {
                    location: "/x/skills/coffee-sandbox".into(),
                    ..e("coffee-sandbox")
                },
                Entry {
                    location: "/x/skills/plan".into(),
                    ..e("plan")
                },
                Entry {
                    location: "/y/other-dir".into(),
                    ..e("odd")
                },
            ],
            None,
        );
        assert!(text.contains("- /x/skills/: coffee-sandbox, plan"));
        assert!(text.contains("odd (/y/other-dir/SKILL.md)"));
        assert_eq!(hidden_index(&[], None), "");
        assert_eq!(tilde("/home/u/x", Some("/home/u")), "~/x");
    }

    #[test]
    fn turns() {
        let c = catalog();
        let set = exposure(5, true);
        let none_must = BTreeSet::new();
        let input = |prompt: &'static str, s: &'static Exposure| TurnInput {
            prompt,
            catalog: Box::leak(c.clone().into_boxed_slice()),
            settings: s,
            cwd_base: "r",
            must_show: Box::leak(Box::new(none_must.clone())),
            home: None,
        };
        let s5: &'static Exposure = Box::leak(Box::new(set.clone()));
        let s5n: &'static Exposure = Box::leak(Box::new(exposure(5, false)));

        let mut st = SessionState::default();
        let mut calls = 0;
        let out = block_on(decide_turn(&input("hi there", s5), &mut st, |_, _| {
            calls += 1;
            std::future::ready(None)
        }));
        assert_eq!(calls, 0);
        assert!(!st.judged);
        assert_eq!(out.listed.len(), c.len());

        let mut st = SessionState::default();
        let out = block_on(decide_turn(
            &input("ssh into coffee and check the browser", s5),
            &mut st,
            scorer(&[("browser-automation", 0.8)]),
        ));
        assert_eq!(
            out.listed.iter().collect::<Vec<_>>(),
            ["browser-automation", "coffee-sandbox"]
        );
        assert!(out.freeze.is_some());
        assert!(out.index.contains("mempalace"));
        assert!(st.judged);

        let mut st = SessionState::default();
        let first = block_on(decide_turn(
            &input("fix the browser tests", s5),
            &mut st,
            scorer(&[("browser-automation", 0.9)]),
        ));
        let second = block_on(decide_turn(
            &input("now search mempalace for the decision", s5n),
            &mut st,
            none,
        ));
        assert_eq!(second.listed, first.listed);
        assert_eq!(second.index, first.index);
        assert_eq!(
            second
                .reveal
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["mempalace"]
        );
        let third = block_on(decide_turn(
            &input("search mempalace again", s5n),
            &mut st,
            none,
        ));
        assert!(third.reveal.is_empty());

        let out = block_on(decide_turn(
            &input("refactor the kanboard daemon", s5),
            &mut SessionState::default(),
            none,
        ));
        assert_eq!(out.listed.len(), c.len());
        assert_eq!(out.freeze.unwrap_or_default()["failOpen"], json!(true));
    }

    #[test]
    fn must_show_and_restore() {
        let catalog: Vec<Entry> = (0..14)
            .map(|i| Entry {
                name: format!("skill-{i}"),
                description: format!("does thing {i}"),
                location: format!("/s/{i}"),
            })
            .collect();
        let low = |_s: String, q: Value| {
            let m: BTreeMap<String, f64> = q
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| (k.clone(), 0.05))
                .collect();
            std::future::ready(Some(answers_from(&m)))
        };
        let def = Exposure::default();
        let mut st = SessionState::default();
        let m1 = BTreeSet::from(["skill-3".to_owned()]);
        let first = block_on(decide_turn(
            &TurnInput {
                prompt: "refactor the parser module please",
                catalog: &catalog,
                settings: &def,
                cwd_base: "r",
                must_show: &m1,
                home: None,
            },
            &mut st,
            low,
        ));
        assert!(first.listed.contains("skill-3"));
        let m2 = BTreeSet::from(["skill-3".to_owned(), "skill-9".to_owned()]);
        let no_recheck = Exposure {
            recheck: false,
            ..def.clone()
        };
        let later = block_on(decide_turn(
            &TurnInput {
                prompt: "now fix the tests",
                catalog: &catalog,
                settings: &no_recheck,
                cwd_base: "r",
                must_show: &m2,
                home: None,
            },
            &mut st,
            low,
        ));
        assert!(later.listed.contains("skill-9"));
        let s = restore_state(&[
            json!({"customType": SKILLS_JUDGED_ENTRY, "data": {"kept": ["plan"], "hidden": [e("mempalace")], "index": "IDX"}}),
            json!({"customType": SKILLS_REVEALED_ENTRY, "data": {"names": ["mempalace"]}}),
        ]);
        assert!(s.judged);
        assert_eq!(s.kept, ["plan"]);
        assert_eq!(s.index, "IDX");
        assert!(s.revealed.contains("mempalace"));
    }

    #[test]
    fn chatter() {
        for t in [
            "hi",
            "Hello there!",
            "thanks so much.",
            "ok",
            "go ahead",
            "/skill:x",
            "LGTM",
        ] {
            assert!(is_chatter(t), "{t}");
        }
        for t in ["fix the parser", "hello, can you fix the build"] {
            assert!(!is_chatter(t), "{t}");
        }
        let m = reveal_message(&[e("x")], Some("/home/u"));
        assert!(m.contains("- x — x skill (~/.agents/skills/x/SKILL.md)"));
    }
}
