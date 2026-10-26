//! `unipi.workflow` — permission gate and plan mode (unipi `packages/workflow`, unipi-port §3.19).
//!
//! | unipi | here |
//! |---|---|
//! | `tool_call` gate: plan enforcement, then rules → read-only → write/edit → bash (jev in auto) → other | [`Workflow::gate`], [`decide`], [`plan::enforce`], [`bash`] |
//! | approval prompt (Allow once / Always allow / Deny / Deny with note) | `approval` dialog (`allow_once`, `allow_always`, `reject_once`, `reject_once` + feedback) |
//! | `/unipi:permission [ask\|auto\|full\|status]`, Alt+M | command + shortcut `unipi.permission.cycle` |
//! | `/unipi:plan [on\|off\|view\|approve]`, Alt+P, `plan_submit` | command + shortcut `unipi.plan.toggle` + tool |
//! | plan review overlay | `custom` dialog (markdown, side-by-side sections, three buttons) |
//! | runners `unipi:plan-enter/-exit/-off`, `unipi:permission-clear-rules` | hidden commands |
//! | `WORKFLOW_STATUS` (sticky), `PLAN_MODE_CHANGED`, `PERMISSION_MODE_CHANGED`, `MODULE_READY` | bus topics |
//! | `permission` settings | [`settings`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod bash;
pub mod decide;
pub mod pathx;
pub mod plan;
pub mod rules;
pub mod settings;

use std::io::Write as _;

use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::session::{Delivery, Provenance};
use oino_extension::sync::SessionMap;
use oino_extension::wit::session as ws;
use unipi_kit::settings::Store;
use unipi_kit::topics::{
    self, ModuleReady, PermissionModeChanged, PlanModeChanged, WorkflowStatus,
};
use unipi_kit::{jev, provenance::Harness};

use crate::decide::{Decision, Deps, JevRisk, Mode, Step};
use crate::plan::{DISCARD, KEEP, PLAN_MESSAGE_TYPE, PLAN_STATE_ENTRY, PLAN_TOOL, PlanState};
use crate::rules::{PermissionRule, RuleDecision, RuleScope, suggest_pattern};
use crate::settings::{CLEAR_RULES, NAMESPACE};

const STATUS_KEY: &str = "unipi-workflow";
const SOURCE: &str = "Plan mode";

/// Outcome of the plan approval prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Approval {
    /// Approved and handed off.
    Approved,
    /// Keep planning (optional feedback).
    Keep(Option<String>),
    /// Discarded; plan mode off.
    Discarded,
}

/// Tool-call fields the gates need (unipi `describeToolCall`).
#[must_use]
pub fn describe_tool_call(tool: &str, input: &Value) -> String {
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match tool {
        "bash" | "powershell" => field("command"),
        "write" | "edit" => field("path"),
        _ => input.to_string().chars().take(300).collect(),
    }
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max - 1).collect::<String>())
    }
}

/// The approval dialog request (ui-protocol §4.3 `approval`).
#[must_use]
pub fn approval_request(tool: &str, summary: &str, reason: &str, pattern: &str) -> Value {
    json!({
        "method": "approval",
        "title": format!("Allow {tool}: {}?\n{}", clip(summary, 120), clip(reason, 120)),
        "toolName": tool,
        "summary": clip(summary, 120),
        "detail": {"t": "text", "value": clip(reason, 120)},
        "options": [
            {"optionId": "allow_once", "label": "Allow once", "kind": "allow_once"},
            {"optionId": "allow_always", "label": format!("Always allow `{}`", clip(pattern, 60)), "kind": "allow_always"},
            {"optionId": "deny", "label": "Deny", "kind": "reject_once"},
            {"optionId": "deny_note", "label": "Deny with note…", "kind": "reject_once"},
        ],
        "feedback": {"placeholder": "Why deny? (sent to the agent)"},
    })
}

/// The extension.
#[derive(Default)]
pub struct Workflow {
    settings: Store,
    plans: SessionMap<PlanState>,
}

fn harness(title: &str) -> Value {
    json!({"oinoProvenance": {"source": SOURCE, "title": title}})
}

impl Workflow {
    fn resolved(&self) -> settings::Resolved {
        settings::resolve(
            &self.settings.get(NAMESPACE),
            settings::read_file_rules(&self.settings),
            &settings::load(RuleScope::Project),
            &settings::load(RuleScope::Global),
        )
    }

    fn debug(&self, line: &str) {
        if !self.resolved().debug {
            return;
        }
        let path = unipi_kit::paths::logs("permission.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "{} {line}",
                unipi_kit::fmt::iso_from_ms(timers::now_ms())
            );
        }
    }

    fn tmpdir() -> String {
        host::env("TMPDIR")
            .ok()
            .flatten()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "/tmp".to_owned())
    }

    fn register_namespace(&self) {
        let ns = settings::namespace(self.resolved().rules.len()).to_json();
        let _ = oino_extension::wit::registry::register_settings(&ns.to_string());
    }

    // ── permission mode ──────────────────────────────────────────────────────

    fn set_mode(&self, mode: Mode) {
        settings::persist_mode(&self.settings, mode.as_str());
        update_status(None, Some(mode.as_str()));
        unipi_kit::bus::emit(&PermissionModeChanged {
            mode: mode.as_str().into(),
        });
        self.debug(&format!("mode set to {}", mode.as_str()));
    }

    fn clear_rules(&self) -> usize {
        let count = self.resolved().rules.len();
        settings::clear_rules(&self.settings);
        self.register_namespace();
        count
    }

    async fn ask_jev(&self, state: &str) -> Option<JevRisk> {
        let all = control::settings()
            .ok()
            .and_then(|s| s.get("extensionSettings").cloned())
            .unwrap_or_else(|| json!({NAMESPACE: self.settings.get(NAMESPACE).0}));
        let js = jev::Settings::resolve(&all, Some(NAMESPACE));
        let questions = json!({"risk": {
            "type": "choice",
            "instructions": "Would running this shell command be safe without asking the user first?",
            "criteria": {
                "safe": "Routine development command with no destructive or external side effects beyond the project (build, test, lint, install deps, run scripts, git add/commit)",
                "needs_approval": "Could modify things outside the project, publish/deploy, delete data, change system/global config, or spend money",
                "dangerous": "Destructive, irreversible, exfiltrates secrets, or escalates privileges",
            },
        }});
        let answers = jev::ask(&js, state, &questions).await?;
        let a = answers.get("risk")?;
        Some(JevRisk {
            choice: a.choice.clone()?,
            confidence: a.confidence.unwrap_or(0.0),
        })
    }

    /// The approval prompt → `Err(reason)` when denied.
    async fn approve(
        &self,
        cx: &Ctx,
        tool: &str,
        summary: &str,
        reason: &str,
        subject: &str,
    ) -> std::result::Result<(), String> {
        let pattern = suggest_pattern(tool, subject);
        let answer = ui::dialog(&approval_request(tool, summary, reason, &pattern))
            .await
            .unwrap_or(Value::Null);
        let value = answer.get("value").unwrap_or(&answer);
        let option = value
            .get("optionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let feedback = value
            .get("feedback")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(ToOwned::to_owned);
        match option {
            "allow_once" => {
                self.debug(&format!("approved tool={tool} rule=-"));
                Ok(())
            }
            "allow_always" => {
                settings::persist_rule(
                    &self.settings,
                    PermissionRule {
                        tool: tool.to_owned(),
                        pattern: pattern.clone(),
                        decision: RuleDecision::Allow,
                        scope: RuleScope::Project,
                    },
                );
                self.register_namespace();
                if cx.has_ui() {
                    let _ = ui::notify(
                        &format!("Permission rule saved: allow {tool} `{pattern}`"),
                        "info",
                    );
                }
                self.debug(&format!("approved tool={tool} rule={pattern}"));
                Ok(())
            }
            other => {
                let note = if other == "deny_note" && feedback.is_none() {
                    ui::input("Why deny? (sent to the agent)", Some(""))
                        .await
                        .ok()
                        .flatten()
                        .map(|n| n.trim().to_owned())
                        .filter(|n| !n.is_empty())
                } else {
                    feedback
                };
                self.debug(&format!(
                    "denied tool={tool} note={}",
                    serde_json::to_string(note.as_deref().unwrap_or("")).unwrap_or_default()
                ));
                Err(match note {
                    Some(n) => format!("Blocked by permission (user denied): {n}"),
                    None => "Blocked by permission (user denied)".to_owned(),
                })
            }
        }
    }

    /// The `tool_call` gate.
    async fn gate(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let tool = ev
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let input = ev.get("input").cloned().unwrap_or(Value::Null);
        let subject = describe_tool_call(&tool, &input);
        let summary = clip(&subject, usize::MAX >> 1);
        let cwd = cx.cwd().display().to_string();
        let tmp = Self::tmpdir();
        let block = |reason: String| Some(json!({"block": true, "reason": reason}));
        let r = self.resolved();
        let state = self.plans.with(cx.session_id(), |s| s.clone());
        match plan::enforce(&tool, &subject, &state, &cwd, &tmp) {
            Some(plan::Enforcement::Block(reason)) => return block(reason),
            Some(plan::Enforcement::Ask(reason)) if r.mode != Mode::Full => {
                if !cx.has_ui() {
                    return block(reason);
                }
                return self
                    .approve(cx, &tool, &summary, &reason, &subject)
                    .await
                    .err()
                    .and_then(block);
            }
            _ => {}
        }
        let deps = Deps {
            mode: r.mode,
            jev_confidence: r.jev_confidence,
            rules: &r.rules,
            cwd: &cwd,
            tmpdir: &tmp,
            has_ui: cx.has_ui(),
        };
        let decision = match decide::decide(&tool, &subject, &deps) {
            Step::Done(d) => d,
            Step::NeedsJev { state, subject } => {
                let risk = self.ask_jev(&state).await;
                decide::after_jev(risk.as_ref(), &subject, &deps)
            }
        };
        let (action, reason) = match &decision {
            Decision::Allow(r) => ("allow", r.clone()),
            Decision::Ask(r, _) => ("ask", r.clone()),
            Decision::Block(r) => ("block", r.clone()),
        };
        self.debug(&format!(
            "decision tool={tool} mode={} action={action} reason={} subject={}",
            r.mode.as_str(),
            serde_json::to_string(&reason).unwrap_or_default(),
            serde_json::to_string(&subject.chars().take(200).collect::<String>())
                .unwrap_or_default()
        ));
        match decision {
            Decision::Allow(_) => None,
            Decision::Block(reason) => block(reason),
            Decision::Ask(reason, subj) => self
                .approve(cx, &tool, &summary, &reason, &subj)
                .await
                .err()
                .and_then(block),
        }
    }

    // ── plan mode ────────────────────────────────────────────────────────────

    fn plan_display(&self, cx: &Ctx) -> String {
        let cwd = cx.cwd().display().to_string();
        let file = self.plans.with(cx.session_id(), |s| s.plan_file.clone());
        plan::display_plan_path(&cwd, file.as_deref())
    }

    fn read_plan(file: Option<&str>) -> Option<String> {
        let guest = pathx::guest_of(&pathx::roots(), file?)?;
        let content = std::fs::read_to_string(guest).ok()?;
        let t = content.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }

    async fn enable_plan(&self, cx: &Ctx) -> String {
        let cwd = cx.cwd().display().to_string();
        let sid = cx.session_id().to_owned();
        let file = self.plans.with(&sid, |s| {
            s.plan_file
                .clone()
                .unwrap_or_else(|| plan::plan_file_path(&cwd, &sid, timers::now_ms()))
        });
        if let Some(dir) = pathx::guest_of(&pathx::roots(), &file)
            .as_deref()
            .and_then(|g| std::path::Path::new(g).parent().map(ToOwned::to_owned))
        {
            let _ = std::fs::create_dir_all(dir);
        }
        self.plans.with(&sid, |s| {
            s.active = true;
            s.plan_file = Some(file.clone());
        });
        let _ = session::append_entry(
            PLAN_STATE_ENTRY,
            Some(&json!({"active": true, "planFile": file})),
        );
        let display = plan::display_plan_path(&cwd, Some(&file));
        update_status(Some(true), None);
        unipi_kit::bus::emit(&PlanModeChanged {
            active: true,
            plan_file: display.clone(),
            reason: None,
        });
        let _ = Harness::new(SOURCE, "Plan mode on")
            .synopsis("Plan with the user — plan file required")
            .send_custom(
                PLAN_MESSAGE_TYPE,
                json!(plan::instructions(&display)),
                Some(harness("Plan mode on")),
            )
            .await;
        if cx.has_ui() {
            let _ = ui::notify(&format!("Plan mode on — plan file {display}"), "info");
        }
        file
    }

    async fn disable_plan(&self, cx: &Ctx, reason: &str) {
        let cwd = cx.cwd().display().to_string();
        let Some(file) = self.plans.with(cx.session_id(), |s| {
            if !s.active {
                return None;
            }
            s.active = false;
            s.last_kept_hash = None;
            Some(s.plan_file.clone())
        }) else {
            return;
        };
        let _ = session::append_entry(
            PLAN_STATE_ENTRY,
            Some(&json!({"active": false, "planFile": file})),
        );
        update_status(Some(false), None);
        unipi_kit::bus::emit(&PlanModeChanged {
            active: false,
            plan_file: plan::display_plan_path(&cwd, file.as_deref()),
            reason: Some(reason.to_owned()),
        });
        if reason != "approved" {
            let (note, title) = if reason == "discarded" {
                ("[plan mode off — plan discarded]", "Plan discarded")
            } else {
                ("[plan mode off]", "Plan mode off")
            };
            let _ = Harness::new(SOURCE, title)
                .send_custom(PLAN_MESSAGE_TYPE, json!(note), Some(harness(title)))
                .await;
        }
        if cx.has_ui() {
            let _ = ui::notify(
                if reason == "discarded" {
                    "Plan mode off — plan discarded"
                } else {
                    "Plan mode off"
                },
                "info",
            );
        }
    }

    #[allow(
        clippy::single_match_else,
        reason = "custom dialog with a select fallback"
    )]
    async fn review_choice(content: &str, path: &str) -> Option<&'static str> {
        match ui::dialog(&plan::review_dialog(content, path)).await {
            Ok(answer) => {
                let v = answer.get("value").unwrap_or(&answer);
                match v.get("actionId").and_then(Value::as_str) {
                    Some("approve") => Some(plan::APPROVE),
                    Some("keep") => Some(KEEP),
                    Some("discard") => Some(DISCARD),
                    _ => None,
                }
            }
            Err(_) => {
                let pick = ui::select(
                    &format!("Plan ready ({path}) — what next?"),
                    &[plan::APPROVE, KEEP, DISCARD],
                )
                .await
                .ok()
                .flatten()?;
                [plan::APPROVE, KEEP, DISCARD]
                    .into_iter()
                    .find(|c| *c == pick)
            }
        }
    }

    /// The approval prompt shared by the tool, `/unipi:plan approve` and the runner.
    async fn approve_plan(&self, cx: &Ctx) -> Approval {
        let sid = cx.session_id().to_owned();
        let file = self.plans.with(&sid, |s| s.plan_file.clone());
        let Some(content) = Self::read_plan(file.as_deref()) else {
            return Approval::Keep(Some(
                "The plan file is empty — write the plan first.".into(),
            ));
        };
        let choice = Self::review_choice(&content, &self.plan_display(cx)).await;
        match choice {
            Some(c) if c == plan::APPROVE => {
                self.disable_plan(cx, "approved").await;
                let text = format!(
                    "Implement the approved plan below. Treat it as authoritative; do not re-plan.\n\n{content}"
                );
                let _ = ws::send_user_message(
                    json!(text).to_string(),
                    ws::UserMessageOptions {
                        deliver_as: Some(Delivery::FollowUp),
                        expand_prompt_templates: false,
                        provenance: Some(Provenance {
                            source: SOURCE.into(),
                            title: Some("Approved plan".into()),
                            synopsis: Some("Plan approved — implementation handoff".into()),
                            severity: None,
                        }),
                    },
                )
                .await;
                Approval::Approved
            }
            Some(c) if c == DISCARD => {
                self.disable_plan(cx, "discarded").await;
                Approval::Discarded
            }
            Some(_) => {
                let feedback = ui::input("What should change in the plan?", Some(""))
                    .await
                    .ok()
                    .flatten()
                    .map(|f| f.trim().to_owned())
                    .filter(|f| !f.is_empty());
                self.plans.with(&sid, |s| {
                    s.last_kept_hash = Some(plan::content_hash(&content));
                });
                Approval::Keep(feedback)
            }
            None => {
                self.plans.with(&sid, |s| {
                    s.last_kept_hash = Some(plan::content_hash(&content));
                });
                Approval::Keep(None)
            }
        }
    }

    async fn toggle_plan(&self, cx: &Ctx) {
        if self.plans.with(cx.session_id(), |s| s.active) {
            self.disable_plan(cx, "toggled").await;
        } else {
            self.enable_plan(cx).await;
        }
    }

    async fn plan_command(&self, args: &str, cx: &Ctx) {
        let action = args.trim().to_lowercase();
        let active = self.plans.with(cx.session_id(), |s| s.active);
        let notify = |m: &str, k: &str| {
            if cx.has_ui() {
                let _ = ui::notify(m, k);
            }
        };
        match action.as_str() {
            "view" => {
                let file = self.plans.with(cx.session_id(), |s| s.plan_file.clone());
                let path = self.plan_display(cx);
                match Self::read_plan(file.as_deref()) {
                    None => notify(&format!("No plan yet at {path}"), "warning"),
                    Some(c) => {
                        let clipped = if c.chars().count() > 1500 {
                            format!("{}\n…", c.chars().take(1500).collect::<String>())
                        } else {
                            c
                        };
                        notify(&format!("{path}\n\n{clipped}"), "info");
                    }
                }
            }
            "approve" => {
                if let Approval::Keep(fb) = self.approve_plan(cx).await {
                    notify(fb.as_deref().unwrap_or("Plan not approved"), "warning");
                }
            }
            "on" if active => notify("Plan mode is already on", "info"),
            "on" => {
                self.enable_plan(cx).await;
            }
            "off" if active => self.disable_plan(cx, "toggled").await,
            "off" => notify("Plan mode is already off", "info"),
            "" => self.toggle_plan(cx).await,
            other => notify(
                &format!("Unknown plan argument \"{other}\" — use on, off, view or approve"),
                "warning",
            ),
        }
    }

    fn permission_command(&self, args: &str, cx: &Ctx) {
        let req = args.trim().to_lowercase();
        let notify = |m: &str, k: &str| {
            if cx.has_ui() {
                let _ = ui::notify(m, k);
            }
        };
        if req.is_empty() || req == "status" {
            let r = self.resolved();
            let default = self
                .settings
                .get(NAMESPACE)
                .get("defaultMode")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_owned();
            notify(
                &format!(
                    "Permission mode: {} (project: {} · default: {default})",
                    r.mode.as_str(),
                    r.project_mode.as_deref().unwrap_or("-")
                ),
                "info",
            );
            return;
        }
        let Some(mode) = Mode::parse(&req) else {
            notify(
                &format!("Unknown permission mode \"{req}\" — use ask, auto or full"),
                "warning",
            );
            return;
        };
        self.set_mode(mode);
        notify(&format!("Permission mode: {req}"), "info");
    }

    async fn plan_submit(&self, cx: &ToolCtx) -> ToolResult {
        let err = |t: String| ToolResult::error(t).with_details(json!({}));
        let (active, file, kept) = self.plans.with(cx.session_id(), |s| {
            (s.active, s.plan_file.clone(), s.last_kept_hash.clone())
        });
        if !active {
            return err("plan_submit is only available in plan mode.".into());
        }
        let Some(content) = Self::read_plan(file.as_deref()) else {
            return err(format!(
                "No plan written yet — write it to {} first.",
                self.plan_display(&cx.cx)
            ));
        };
        if let Some(refusal) = plan::submit_refusal(&content, kept.as_deref()) {
            return err(refusal);
        }
        if !cx.cx.has_ui() {
            return err("plan_submit needs an interactive session for approval.".into());
        }
        match self.approve_plan(&cx.cx).await {
            Approval::Approved => ToolResult::text("Plan approved.")
                .with_details(json!({}))
                .terminate(),
            Approval::Discarded => ToolResult::text("Plan discarded; plan mode is off.")
                .with_details(json!({}))
                .terminate(),
            Approval::Keep(fb) => ToolResult::text(format!(
                "Not approved yet — keep planning.{}",
                fb.map(|f| format!("\nFeedback: {f}")).unwrap_or_default()
            ))
            .with_details(json!({})),
        }
    }

    fn on_session_start(&self, cx: &Ctx) {
        self.settings.refresh();
        if !cx.is_child() {
            let imported = settings::import_legacy(&self.settings);
            if imported > 0 {
                self.debug(&format!("imported {imported} saved rules into settings"));
            }
        }
        let cwd = cx.cwd().display().to_string();
        let entries = session::entries().unwrap_or_default();
        let state = plan::restore(&entries, &cwd, cx.session_id(), timers::now_ms());
        let display = plan::display_plan_path(&cwd, state.plan_file.as_deref());
        let active = state.active;
        self.plans.with(cx.session_id(), |s| *s = state);
        let mode = self.resolved().mode;
        self.register_namespace();
        update_status(Some(active), Some(mode.as_str()));
        unipi_kit::bus::emit(&PermissionModeChanged {
            mode: mode.as_str().into(),
        });
        unipi_kit::bus::emit(&PlanModeChanged {
            active,
            plan_file: display,
            reason: None,
        });
        unipi_kit::bus::emit(&ModuleReady {
            name: "@pi-unipi/workflow".into(),
            version: host::info().extension_version,
            commands: vec!["unipi:plan".into(), "unipi:permission".into()],
            tools: vec![PLAN_TOOL.into()],
            load_time_ms: None,
        });
        if cx.has_ui() {
            let _ = ui::status(STATUS_KEY, None);
        }
    }

    fn before_agent_start(&self, cx: &Ctx) -> Option<Value> {
        if !self.plans.with(cx.session_id(), |s| s.active) {
            return None;
        }
        Some(json!({"message": {
            "customType": PLAN_MESSAGE_TYPE,
            "content": plan::reminder(&self.plan_display(cx)),
            "display": false,
            "details": harness("Plan reminder (hidden)"),
        }}))
    }
}

/// Patches the sticky `WORKFLOW_STATUS` snapshot.
fn update_status(plan_mode: Option<bool>, permission_mode: Option<&str>) {
    let mut cur: WorkflowStatus = unipi_kit::bus::get().unwrap_or_default();
    if let Some(p) = plan_mode {
        cur.plan_mode = p;
    }
    if let Some(m) = permission_mode {
        cur.permission_mode = Some(m.to_owned());
    }
    unipi_kit::bus::emit(&cur);
}

fn completions(prefix: &str, items: &[(&str, &str)]) -> Vec<CompletionItem> {
    let needle = prefix.trim().to_lowercase();
    items
        .iter()
        .filter(|(v, _)| v.starts_with(&needle))
        .map(|(v, d)| CompletionItem {
            value: (*v).to_owned(),
            label: Some((*v).to_owned()),
            description: Some((*d).to_owned()),
        })
        .collect()
}

impl Extension for Workflow {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.settings.declare(NAMESPACE, settings::defaults());
        self.settings.load(cx.settings());
        cx.settings_namespace(&settings::namespace(0).to_json());

        cx.on(EventKind::ToolCall, |ext, ev, cx| async move {
            Ok(ext.gate(&ev, &cx).await)
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.on_session_start(&cx);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.plans.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::BeforeAgentStart, |ext, _ev, cx| async move {
            Ok(ext.before_agent_start(&cx))
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext.settings.on_changed(&ev.payload).as_deref() == Some(NAMESPACE) {
                let keys: Vec<String> = ev
                    .get("keys")
                    .and_then(|k| serde_json::from_value(k.clone()).ok())
                    .unwrap_or_default();
                if keys.iter().any(|k| k == "mode") {
                    // A hub edit of the project mode supersedes the stored one.
                    let mut s = settings::load(RuleScope::Project);
                    s.mode = None;
                    settings::save(RuleScope::Project, &s);
                }
                update_status(None, Some(ext.resolved().mode.as_str()));
            }
            Ok(())
        });

        cx.command(
            Command::<Self>::new("unipi:permission")
                .description("Permission mode — ask · auto (jev-judged) · full")
                .run(|ext, args, cx| async move {
                    ext.permission_command(&args, &cx.cx);
                    Ok(())
                })
                .complete(|_ext, prefix, _cx| async move {
                    completions(
                        &prefix,
                        &[
                            ("ask", "Prompt before every write, bash call and other tool"),
                            (
                                "auto",
                                "Read-only and workspace writes run; jev judges ambiguous bash (default)",
                            ),
                            ("full", "Run everything — only saved deny rules block"),
                        ],
                    )
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:plan")
                .description("Plan mode — plan with the user, then approve a plan")
                .run(|ext, args, cx| async move {
                    ext.plan_command(&args, &cx.cx).await;
                    Ok(())
                })
                .complete(|_ext, prefix, _cx| async move {
                    completions(
                        &prefix,
                        &[
                            ("on", "Enter plan mode"),
                            ("off", "Leave plan mode"),
                            ("view", "Show the current plan file"),
                            ("approve", "Run the plan approval prompt"),
                        ],
                    )
                }),
        );
        cx.command(
            Command::<Self>::new(CLEAR_RULES)
                .description("Clear saved permission rules")
                .hidden()
                .run(|ext, _args, cx| async move {
                    let removed = ext.clear_rules();
                    if cx.has_ui() {
                        let _ = ui::notify(
                            &if removed == 0 {
                                "No saved permission rules".to_owned()
                            } else {
                                format!(
                                    "Cleared {removed} permission rule{}",
                                    if removed == 1 { "" } else { "s" }
                                )
                            },
                            "info",
                        );
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:plan-off")
                .description("Leave plan mode")
                .hidden()
                .run(|ext, _args, cx| async move {
                    ext.disable_plan(&cx.cx, "toggled").await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:plan-enter")
                .description("Enter plan mode (runner)")
                .hidden()
                .run_value(|ext, _args, cx| async move {
                    if ext.plans.with(cx.session_id(), |s| s.active) {
                        return Ok(Some(json!({"ok": true, "alreadyActive": true})));
                    }
                    let file = ext.enable_plan(&cx.cx).await;
                    Ok(Some(json!({"ok": true, "planFile": file})))
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:plan-exit")
                .description("Leave plan mode (runner)")
                .hidden()
                .run_value(|ext, _args, cx| async move {
                    ext.disable_plan(&cx.cx, "toggled").await;
                    Ok(Some(json!({"ok": true})))
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.plan.toggle", &["alt+p"])
                .description("Toggle plan mode")
                .run(|ext, cx| async move {
                    ext.toggle_plan(&cx.cx).await;
                    Ok(())
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.permission.cycle", &["alt+m"])
                .description("Cycle permission mode (ask → auto → full)")
                .run(|ext, cx| async move {
                    let next = ext.resolved().mode.next();
                    ext.set_mode(next);
                    if cx.has_ui() {
                        let label = match next {
                            Mode::Ask => "ask (always prompt)",
                            Mode::Auto => "auto (jev judges ambiguous bash)",
                            Mode::Full => "full (only deny rules block)",
                        };
                        let _ = ui::notify(&format!("Permission mode: {label}"), "info");
                    }
                    Ok(())
                }),
        );
        cx.tool(
            Tool::<Self>::new(PLAN_TOOL)
                .label("Submit Plan")
                .description("Plan mode only: submit the finished plan in <plan file> for approval, after every shaping question is settled with the user. Refused while ### Open questions lists items or when the plan is unchanged since the user asked to keep planning.")
                .prompt_snippet("Submit the plan for approval (plan mode only).")
                .parameters(&json!({"type": "object", "properties": {}}))
                .kind(ToolKind::SwitchMode)
                .run(|ext, _args: Value, cx| async move { Ok(ext.plan_submit(&cx).await) }),
        );
        let _ = topics::WORKFLOW_STATUS;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Workflow);

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
    use super::*;

    #[test]
    fn describe() {
        assert_eq!(describe_tool_call("bash", &json!({"command": "ls"})), "ls");
        assert_eq!(describe_tool_call("edit", &json!({"path": "a.ts"})), "a.ts");
        assert_eq!(describe_tool_call("x", &json!({"a": 1})), "{\"a\":1}");
        assert_eq!(describe_tool_call("bash", &json!({})), "");
    }

    #[test]
    fn approval_shape() {
        let r = approval_request(
            "bash",
            "rm -rf /tmp/wd-test",
            "dangerous: rm -rf",
            &suggest_pattern("bash", "rm -rf /tmp/wd-test"),
        );
        let labels: Vec<&str> = r["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["label"].as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "Allow once",
                "Always allow `rm -rf /tmp/wd-test`",
                "Deny",
                "Deny with note…"
            ]
        );
        assert!(
            r["title"]
                .as_str()
                .unwrap_or_default()
                .starts_with("Allow bash: rm -rf /tmp/wd-test?\n")
        );
        assert_eq!(r["options"][2]["kind"], "reject_once");
        assert_eq!(clip("a  b\nc", 120), "a b c");
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
