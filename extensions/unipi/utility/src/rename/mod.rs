//! Automatic session naming (unipi `src/rename/index.ts`).
//!
//! After every confirmed round (`agent_end` without error/abort) whose prompt the user typed:
//! gate (prefilter + jev, [`gate`]) → isolated one-tool naming session ([`session`]) →
//! `session.set-session-name`. Names the user sets (`/name`) are respected: once the current
//! name differs from the last auto name, auto-rename stops for that session. The hidden runner
//! `rename-now` (settings hub "Rename now") bypasses the gate. Herdr pane sync is not
//! ported (D22).

pub mod gate;
pub mod session;

use std::cell::RefCell;
use std::time::Duration;

use futures::StreamExt;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use serde_json::Value;

use self::gate::{
    GateInput, count_tool_calls, decide, gate_request, is_idle_round, last_reply_text,
};
use self::session::{RenameRequest, candidates, name_from_event, name_from_message};

/// Custom entry recording names auto-rename set (so resume can tell them from `/name`).
pub const AUTO_NAME_ENTRY: &str = "unipi:auto-name";
/// Hidden runner of the settings hub action.
pub const RUNNER: &str = "rename-now";
/// Rounds that must pass between two automatic renames.
pub const MIN_ROUNDS_BETWEEN: u32 = 1;
const MAX_REQUESTS: usize = 6;

/// Per-session naming state.
#[derive(Clone, Debug)]
pub struct State {
    /// Last typed requests (oldest first, ≤ 6).
    pub requests: Vec<String>,
    /// Prompt of the running round.
    pub pending: Option<String>,
    /// Name auto-rename set last.
    pub last_auto_name: Option<String>,
    /// Rounds since the last automatic rename.
    pub rounds_since_rename: u32,
    /// A naming session is running.
    pub in_flight: bool,
    /// The round that triggered the current rename: `(reply, tool calls)`.
    pub last_round: (String, usize),
}

impl Default for State {
    fn default() -> Self {
        Self {
            requests: Vec::new(),
            pending: None,
            last_auto_name: None,
            rounds_since_rename: u32::MAX,
            in_flight: false,
            last_round: (String::new(), 0),
        }
    }
}

impl State {
    /// `session_start`: restores the last auto name from the entries.
    pub fn reset(&mut self, entries: &[Value]) {
        *self = Self {
            last_auto_name: last_auto_name(entries),
            ..Self::default()
        };
    }

    /// `input`: records typed, non-command prompts.
    pub fn on_input(&mut self, text: &str, source: Option<&str>) {
        if source == Some("extension") {
            return;
        }
        let t = text.trim();
        if t.is_empty() || t.starts_with('/') {
            return;
        }
        self.requests.push(t.to_owned());
        let excess = self.requests.len().saturating_sub(MAX_REQUESTS);
        self.requests.drain(..excess);
        self.pending = Some(t.to_owned());
    }

    /// `agent_end`: the gate input when this round may rename, else the skip reason.
    ///
    /// # Errors
    ///
    /// The skip reason (debug trail) when the round cannot rename.
    pub fn on_agent_end(
        &mut self,
        messages: &[Value],
        auto: bool,
        current_name: Option<&str>,
    ) -> Result<GateInput, String> {
        let prompt = self.pending.take();
        self.rounds_since_rename = self.rounds_since_rename.saturating_add(1);
        let Some(prompt) = prompt else {
            return Err("no typed prompt".into());
        };
        if self.in_flight {
            return Err("rename in flight".into());
        }
        if !auto {
            return Err("auto-rename off".into());
        }
        let stop = messages
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
            .and_then(|m| m.get("stopReason").and_then(Value::as_str));
        if let Some(reason @ ("error" | "aborted")) = stop {
            return Err(format!("round {reason}"));
        }
        if let Some(name) = current_name {
            if self.last_auto_name.as_deref() != Some(name) {
                return Err("user-set name".into());
            }
            if self.rounds_since_rename < MIN_ROUNDS_BETWEEN {
                return Err("renamed recently".into());
            }
        }
        let input = GateInput {
            prompt,
            current_name: current_name.map(ToOwned::to_owned),
            earlier: self.requests.clone(),
            reply: last_reply_text(messages),
            tool_calls: count_tool_calls(messages),
        };
        if is_idle_round(&input) {
            return Err("chatter".into());
        }
        self.last_round = (input.reply.clone(), input.tool_calls);
        Ok(input)
    }

    /// Records an applied automatic name.
    pub fn applied(&mut self, name: &str) {
        self.last_auto_name = Some(name.to_owned());
        self.rounds_since_rename = 0;
    }
}

/// Last `unipi:auto-name` entry's name.
#[must_use]
pub fn last_auto_name(entries: &[Value]) -> Option<String> {
    entries
        .iter()
        .rev()
        .find(|e| {
            e.get("type").and_then(Value::as_str) == Some("custom")
                && e.get("customType").and_then(Value::as_str) == Some(AUTO_NAME_ENTRY)
        })
        .and_then(|e| e.get("data")?.get("name")?.as_str().map(ToOwned::to_owned))
}

/// Auto-rename over the host.
#[derive(Default)]
pub struct Renamer {
    sessions: SessionMap<State>,
    /// Name chosen by the child's `rename_session` call (fallback to event scanning).
    chosen: RefCell<Option<String>>,
}

fn session_model() -> Option<String> {
    let m = control::model()?;
    Some(format!(
        "{}/{}",
        m.get("provider")?.as_str()?,
        m.get("id")?.as_str()?
    ))
}

impl Renamer {
    /// `session_start`.
    pub fn on_session_start(&self, sid: &str) {
        let entries = oino_extension::session::entries().unwrap_or_default();
        self.sessions.with(sid, |s| s.reset(&entries));
    }

    /// `session_shutdown`.
    pub fn on_session_shutdown(&self, sid: &str) {
        self.sessions.remove(sid);
    }

    /// `input`.
    pub fn on_input(&self, sid: &str, ev: &Event) {
        let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
        let source = ev.get("source").and_then(Value::as_str);
        self.sessions.with(sid, |s| s.on_input(text, source));
    }

    /// The child's `rename_session` call.
    pub fn record_choice(&self, name: &str) {
        *self.chosen.borrow_mut() = Some(name.to_owned());
    }

    /// `agent_end`: gate (detached), then rename.
    pub fn on_agent_end(&'static self, utility: &'static crate::Utility, sid: &str, ev: &Event) {
        let messages: Vec<Value> = ev
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let settings = utility.util();
        let current = oino_extension::session::session_name();
        let gated = self.sessions.with(sid, |s| {
            s.on_agent_end(&messages, settings.rename.auto, current.as_deref())
        });
        let input = match gated {
            Ok(i) => i,
            Err(why) => {
                if why != "no typed prompt" {
                    utility.debug_log("rename.log", &format!("skip: {why}"));
                }
                return;
            }
        };
        let sid = sid.to_owned();
        crate::spawn(async move {
            let (state, questions) = gate_request(&input);
            let js = crate::jev_settings();
            let answers = unipi_kit::jev::ask(&js, &state, &questions).await;
            let decision = decide(&input, answers.as_ref());
            let preview: String = input.prompt.chars().take(60).collect();
            utility.debug_log(
                "rename.log",
                &format!(
                    "gate {preview:?} name={:?} → {} ({})",
                    input.current_name,
                    if decision.rename { "rename" } else { "skip" },
                    decision.reason
                ),
            );
            if decision.rename {
                let _ = self
                    .rename(utility, &sid, false, input.current_name.is_some())
                    .await;
            }
        });
    }

    /// Runs the naming session and applies its name; returns the chosen name.
    pub async fn rename(
        &self,
        utility: &crate::Utility,
        sid: &str,
        force: bool,
        topic_changed: bool,
    ) -> Option<String> {
        let started = self.sessions.with(sid, |s| {
            if s.in_flight {
                false
            } else {
                s.in_flight = true;
                true
            }
        });
        if !started {
            return None;
        }
        let req = self.sessions.with(sid, |s| RenameRequest {
            current_name: oino_extension::session::session_name(),
            requests: s.requests.clone(),
            model: utility.util().rename.model,
            topic_changed,
            reply: s.last_round.0.clone(),
        });
        let name = self.run_session(utility, &req).await;
        utility.debug_log("rename.log", &format!("rename session → {name:?}"));
        if let Some(n) = &name
            && (force || Some(n.as_str()) != req.current_name.as_deref())
        {
            self.apply(sid, n);
        }
        self.sessions.with(sid, |s| s.in_flight = false);
        name
    }

    fn apply(&self, sid: &str, name: &str) {
        if oino_extension::session::session_name().as_deref() == Some(name) {
            return;
        }
        if oino_extension::session::set_session_name(Some(name)).is_ok() {
            self.sessions.with(sid, |s| s.applied(name));
            let _ = oino_extension::session::append_entry(
                AUTO_NAME_ENTRY,
                Some(&json!({"name": name})),
            );
        }
    }

    async fn run_session(&self, utility: &crate::Utility, req: &RenameRequest) -> Option<String> {
        for model in candidates(&req.model, session_model().as_deref()) {
            match self.via_agent(req, model.as_deref()).await {
                Ok(Some(name)) => return Some(name),
                Ok(None) => {
                    utility.debug_log("rename.log", &format!("{model:?}: no rename_session call"));
                }
                Err(e) => {
                    utility.debug_log(
                        "rename.log",
                        &format!("nested agent unavailable ({e}); one-shot completion"),
                    );
                    match models::complete(&session::completion_request(req, model.as_deref()))
                        .await
                    {
                        Ok(msg) => {
                            if let Some(name) = name_from_message(&msg) {
                                return Some(name);
                            }
                            utility.debug_log(
                                "rename.log",
                                &format!("{model:?}: completion without rename_session"),
                            );
                        }
                        Err(e) => utility
                            .debug_log("rename.log", &format!("{model:?}: completion failed: {e}")),
                    }
                }
            }
        }
        None
    }

    async fn via_agent(&self, req: &RenameRequest, model: Option<&str>) -> Result<Option<String>> {
        *self.chosen.borrow_mut() = None;
        let agent = agents::Agent::spawn(&session::agent_spec(model)).await?;
        let mut events = agent.events();
        let mut found: Option<String> = None;
        let outcome = agent.prompt(&session::rename_prompt(req)).await;
        if outcome.is_ok() {
            let idle = agent.wait_idle();
            let timeout = timers::sleep(Duration::from_millis(session::TIMEOUT_MS));
            futures::pin_mut!(idle, timeout);
            loop {
                let next = events.next();
                futures::pin_mut!(next);
                let either = futures::future::select(
                    idle.as_mut(),
                    futures::future::select(timeout.as_mut(), next),
                );
                match either.await {
                    futures::future::Either::Left(_) => break,
                    futures::future::Either::Right((futures::future::Either::Left(_), _)) => {
                        agent.abort();
                        break;
                    }
                    futures::future::Either::Right((
                        futures::future::Either::Right((ev, _)),
                        _,
                    )) => {
                        let Some(ev) = ev else {
                            break;
                        };
                        if found.is_none() {
                            found = name_from_event(&ev);
                        }
                    }
                }
            }
        }
        agent.close(1_000).await;
        outcome?;
        Ok(found.or_else(|| self.chosen.borrow_mut().take()))
    }

    /// `rename-now`.
    pub async fn rename_now(&self, utility: &crate::Utility, sid: &str) -> Result<()> {
        if self.sessions.with(sid, |s| s.requests.is_empty()) {
            let _ = ui::notify("Nothing to name yet — send a request first.", "info");
            return Ok(());
        }
        match self.rename(utility, sid, true, false).await {
            Some(name) => ui::notify(&format!("Session named \"{name}\""), "info")?,
            None => ui::notify("Could not generate a name.", "warning")?,
        }
        Ok(())
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

    fn assistant(stop: &str, text: &str) -> Value {
        json!({"role": "assistant", "stopReason": stop, "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn input_tracking() {
        let mut s = State::default();
        s.on_input("  /plan ", None);
        s.on_input("", None);
        s.on_input("from an extension", Some("extension"));
        assert!(s.requests.is_empty() && s.pending.is_none());
        for i in 0..8 {
            s.on_input(&format!("request number {i}"), Some("interactive"));
        }
        assert_eq!(s.requests.len(), 6);
        assert_eq!(s.requests[0], "request number 2");
        assert_eq!(s.pending.as_deref(), Some("request number 7"));
    }

    #[test]
    fn agent_end_gating() {
        let mut s = State::default();
        let msgs = vec![assistant("stop", "done")];
        assert_eq!(
            s.on_agent_end(&msgs, true, None),
            Err("no typed prompt".into())
        );
        s.on_input("fix the lane ordering bug", None);
        assert_eq!(
            s.on_agent_end(&msgs, false, None),
            Err("auto-rename off".into())
        );
        s.on_input("fix the lane ordering bug", None);
        assert_eq!(
            s.on_agent_end(&[assistant("aborted", "")], true, None),
            Err("round aborted".into())
        );
        s.on_input("fix the lane ordering bug", None);
        assert_eq!(
            s.on_agent_end(&msgs, true, Some("Mine")),
            Err("user-set name".into())
        );
        s.on_input("thanks", None);
        assert_eq!(s.on_agent_end(&msgs, true, None), Err("chatter".into()));
        s.on_input("fix the lane ordering bug", None);
        let input = s.on_agent_end(&msgs, true, None).unwrap_or_default();
        assert_eq!(input.prompt, "fix the lane ordering bug");
        assert_eq!(input.reply, "done");
        assert_eq!(s.last_round.0, "done");
    }

    #[test]
    fn auto_names_are_renamed_again_but_not_twice_in_a_round() {
        let mut s = State::default();
        s.applied("Auto Name");
        s.on_input("now a different thing entirely", None);
        // `agent_end` counts this round first, so one round has passed.
        assert!(
            s.on_agent_end(&[assistant("stop", "x")], true, Some("Auto Name"))
                .is_ok()
        );
        s.in_flight = true;
        s.on_input("another task to name please", None);
        assert_eq!(
            s.on_agent_end(&[], true, Some("Auto Name")),
            Err("rename in flight".into())
        );
    }

    #[test]
    fn restores_last_auto_name() {
        let entries = vec![
            json!({"type": "custom", "customType": AUTO_NAME_ENTRY, "data": {"name": "First"}}),
            json!({"type": "message"}),
            json!({"type": "custom", "customType": AUTO_NAME_ENTRY, "data": {"name": "Second"}}),
        ];
        let mut s = State::default();
        s.on_input("x y z w", None);
        s.reset(&entries);
        assert_eq!(s.last_auto_name.as_deref(), Some("Second"));
        assert!(s.requests.is_empty());
        assert_eq!(last_auto_name(&[]), None);
    }
}
