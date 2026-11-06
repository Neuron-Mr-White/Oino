//! Host-owned permission choke point. Tool declarations and child profiles are not consent.

use super::{AgentSession, lock};
use oino_ai::{JsonObject, JsonValue};
use oino_settings::autonomy::{self, Effect, Permissions, Rule, Scope};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

pub use oino_settings::autonomy::Level;

/// Correlated approval emitted before execution, never by the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRequest {
    /// Unique, single-use correlation id.
    pub request_id: String,
    /// Session whose call is waiting.
    pub session_id: String,
    /// Canonical tool name.
    pub tool: String,
    /// Canonical execution arguments.
    pub arguments: JsonObject,
    /// Effective shell risk (non-shell tools have no risk ceiling).
    pub risk_level: Option<Level>,
    /// Effective session ceiling.
    pub ceiling: Level,
    /// Actionable approval reason.
    pub reason: String,
}
/// A frontend's decision, never an extension/model authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// This pending invocation only.
    AllowOnce,
    /// Save one project-scoped exact shell command, interactive only.
    Always,
    /// Abort this invocation.
    Deny,
}
/// Response scoped to the backend session on which it arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalResponse {
    /// Pending request id.
    pub request_id: String,
    /// Decision.
    pub decision: ApprovalDecision,
}
struct Pending {
    tx: oneshot::Sender<ApprovalDecision>,
}
/// Per-session state. All frontends start without approval capability.
#[derive(Default)]
pub(crate) struct PermissionState {
    level: Option<Level>,
    unattended: bool,
    approval_ui: bool,
    pending: HashMap<String, Pending>,
}
struct PendingGuard<'a> {
    state: &'a Mutex<PermissionState>,
    id: String,
}
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        lock(self.state).pending.remove(&self.id);
    }
}

impl AgentSession {
    /// Set a session/ticket ceiling. Children can never exceed their parent's ceiling.
    pub fn set_autonomy(&self, level: Level) {
        lock(&self.inner.permission).level = Some(level);
        let _ = self.with_manager(|m| {
            m.append_custom_entry("oino:autonomy", Some(serde_json::json!({"level":level})))
        });
    }
    /// Global default applies unless the host has explicitly set a session/ticket ceiling.
    #[must_use]
    pub fn autonomy(&self) -> Level {
        let own = lock(&self.inner.permission).level.unwrap_or_else(|| {
            self.with_manager(|m| {
                m.branch(None)
                    .into_iter()
                    .rev()
                    .find_map(|entry| match &entry.kind {
                        oino_session::EntryKind::Custom { custom_type, data }
                            if custom_type == "oino:autonomy" =>
                        {
                            data.as_ref()
                                .and_then(|v| v.get("level"))
                                .and_then(|v| serde_json::from_value(v.clone()).ok())
                        }
                        _ => None,
                    })
            })
            .unwrap_or_else(|| {
                self.with_settings(|s| autonomy::default_level(s.snapshot().global()))
            })
        });
        match self
            .inner
            .agents
            .link
            .as_ref()
            .and_then(|l| l.parent.upgrade())
        {
            Some(parent) => own.min(Self::from_inner(parent).autonomy()),
            None if self.is_child() => Level::Low,
            None => own,
        }
    }
    /// Mark cron, dispatcher workers and background agents unattended. This is sticky.
    /// Disabling the approval UI also cancels all pending approvals.
    pub fn set_unattended(&self) {
        let _ = self.with_manager(|m| m.append_custom_entry("oino:unattended", None));
        let mut state = lock(&self.inner.permission);
        state.unattended = true;
        state.approval_ui = false;
        state.pending.clear();
    }
    /// Only a transport with an actual approval implementation may opt in.
    /// Tool availability, extension UI presence and child profiles never opt in.
    pub fn set_approval_ui(&self, available: bool) {
        let mut state = lock(&self.inner.permission);
        state.approval_ui = available && !state.unattended;
        if !state.approval_ui {
            state.pending.clear();
        }
    }
    fn approval_owner(&self) -> Option<Self> {
        let state = lock(&self.inner.permission);
        let persisted = self.with_manager(|m| m.branch(None).iter().any(|e| matches!(&e.kind, oino_session::EntryKind::Custom { custom_type, .. } if custom_type == "oino:unattended")));
        if state.unattended || persisted {
            return None;
        }
        if let Some(link) = &self.inner.agents.link {
            if !matches!(link.ui, crate::agents::UiPolicy::Forward) {
                return None;
            }
            let parent = link.parent.upgrade()?;
            drop(state);
            return Self::from_inner(parent).approval_owner();
        }
        state.approval_ui.then(|| self.clone())
    }
    /// Consume a pending response exactly once. Wrong-session, stale, duplicate and
    /// unattended responses are errors. The session owning the parent UI owns child requests.
    ///
    /// # Errors
    /// No pending interactive request with this id, or its waiter was cancelled.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "wire responses are single-use owned messages"
    )]
    pub fn approval_response(&self, response: ApprovalResponse) -> Result<(), String> {
        let mut state = lock(&self.inner.permission);
        if state.unattended || !state.approval_ui {
            return Err("approval response on unattended session".into());
        }
        state
            .pending
            .remove(&response.request_id)
            .ok_or("stale or wrong-session approval response")?
            .tx
            .send(response.decision)
            .map_err(|_| "approval request cancelled".into())
    }
    pub(crate) fn cancel_approvals(&self) {
        lock(&self.inner.permission).pending.clear();
    }

    fn permission_project(&self) -> String {
        std::fs::canonicalize(&self.inner.cwd)
            .unwrap_or_else(|_| self.inner.cwd.clone())
            .to_string_lossy()
            .into_owned()
    }
    fn permission_rules(&self) -> Result<Permissions, String> {
        let settings = self.with_settings(|s| s.snapshot());
        let mut combined = Permissions::default();
        for layer in [settings.global(), settings.project()] {
            if let Some(value) = layer.get("permissions") {
                let envelope: Permissions = serde_json::from_value(value.clone())
                    .map_err(|e| format!("invalid permissions: {e}"))?;
                envelope.validate()?;
                combined.default = envelope.default;
                combined.rules.extend(envelope.rules);
                combined.legacy_deny.extend(envelope.legacy_deny);
            }
            // Old allows must never become ceiling bypasses. Old denies remain effective.
            if let Some(rules) = layer
                .get("extensionSettings")
                .and_then(|v| v.get("permission"))
                .and_then(|v| v.get("rules"))
                .and_then(JsonValue::as_array)
            {
                for rule in rules {
                    if rule.get("decision").and_then(JsonValue::as_str) == Some("deny") {
                        let tool = rule
                            .get("tool")
                            .and_then(JsonValue::as_str)
                            .filter(|s| !s.is_empty())
                            .ok_or("invalid legacy deny tool")?;
                        let pattern = rule
                            .get("pattern")
                            .and_then(JsonValue::as_str)
                            .filter(|s| !s.is_empty())
                            .ok_or("invalid legacy deny pattern")?;
                        combined.legacy_deny.push(autonomy::LegacyDeny {
                            tool: tool.into(),
                            pattern: pattern.into(),
                        });
                    }
                }
            }
        }
        combined.validate()?;
        Ok(combined)
    }
    fn save_exact_approval(&self, tool: &str, command: &str) -> Result<(), String> {
        let canonical = autonomy::canonical_command(command)?;
        let project = self.permission_project();
        self.with_settings(|manager| {
            manager.append_permission_grant(Rule {
                id: uuid::Uuid::new_v4().to_string(),
                scope: Scope::Project {
                    project_id: project,
                },
                tool: tool.into(),
                pattern: canonical,
                effect: Effect::Allow,
                interactive_above_ceiling: true,
            })
        })
    }

    /// Evaluate canonical execution arguments and wait only when a real approval
    /// transport has explicitly opted in. Every caller (including fake cron callers)
    /// uses this same public policy API.
    ///
    /// # Errors
    /// Denied rule, malformed input, missing UI, cancellation or rejected approval.
    #[allow(
        clippy::too_many_lines,
        reason = "ordered fail-closed gate and single-use approval transaction"
    )]
    pub async fn check_permission(&self, tool: &str, args: &JsonObject) -> Result<(), String> {
        let args = oino_tools::canonical_args(tool, args).map_err(|e| e.to_string())?;
        let shell = matches!(tool, "bash" | "powershell");
        let risk = if shell {
            let declared: Level = serde_json::from_value(
                args.get("riskLevel")
                    .cloned()
                    .ok_or("riskLevel is required")?,
            )
            .map_err(|_| "invalid riskLevel")?;
            let command = args
                .get("command")
                .and_then(JsonValue::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or("command must be nonempty")?;
            let prefix =
                self.with_settings(|s| s.snapshot().shell_command_prefix().map(str::to_owned));
            let effective = prefix
                .filter(|p| !p.is_empty())
                .map_or_else(|| command.to_owned(), |p| format!("{p}\n{command}"));
            Some(declared.max(if tool == "powershell" {
                autonomy::powershell_floor(&effective)
            } else {
                autonomy::bash_floor(&effective)
            }))
        } else {
            None
        };
        let subjects = policy_subjects(tool, &args, &self.work_cwd())?;
        let ceiling = self.autonomy();
        let owner = self.approval_owner();
        let rules = self.permission_rules()?;
        let (effect, exact) =
            rules.resolve(&self.permission_project(), tool, &subjects, owner.is_some());
        // Patches are file mutations: deny/ask rules for write or edit must not
        // disappear merely because the model selects the patch tool.
        let (effect, exact) = if tool == "apply_patch" {
            let effects = [
                effect,
                rules
                    .resolve(
                        &self.permission_project(),
                        "write",
                        &subjects,
                        owner.is_some(),
                    )
                    .0,
                rules
                    .resolve(
                        &self.permission_project(),
                        "edit",
                        &subjects,
                        owner.is_some(),
                    )
                    .0,
            ];
            (
                if effects.contains(&Effect::Deny) {
                    Effect::Deny
                } else if effects.contains(&Effect::Ask) {
                    Effect::Ask
                } else {
                    effect
                },
                false,
            )
        } else {
            (effect, exact)
        };
        let has_prefix = self.with_settings(|s| {
            s.snapshot()
                .shell_command_prefix()
                .is_some_and(|p| !p.is_empty())
        });
        let exact = exact && !has_prefix;
        if effect == Effect::Deny {
            return Err("blocked by permission deny rule".into());
        }
        let above = risk.is_some_and(|r| r > ceiling);
        if effect != Effect::Ask && (!above || exact) {
            return Ok(());
        }
        let reason = risk.filter(|_| above).map_or_else(
            || "needs interactive permission approval".into(),
            |r| format!("needs Autonomy {}", r.title()),
        );
        let Some(owner) = owner else {
            return Err(reason);
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut state = lock(&owner.inner.permission);
            if state.unattended || !state.approval_ui {
                return Err(reason);
            }
            state.pending.insert(id.clone(), Pending { tx });
        }
        let _guard = PendingGuard {
            state: &owner.inner.permission,
            id: id.clone(),
        };
        owner
            .inner
            .hub
            .publish(crate::events::OinoEvent::ApprovalRequest {
                request: ApprovalRequest {
                    request_id: id,
                    session_id: self.session_id(),
                    tool: tool.into(),
                    arguments: args.clone(),
                    risk_level: risk,
                    ceiling,
                    reason: reason.clone(),
                },
            });
        let decision = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| reason.clone())?;
        // A disconnect or background transition invalidates grants even if a response raced it.
        if self.approval_owner().is_none() {
            return Err(reason);
        }
        let current = self.permission_rules()?;
        for name in if tool == "apply_patch" {
            vec![tool, "write", "edit"]
        } else {
            vec![tool]
        } {
            if current
                .resolve(&self.permission_project(), name, &subjects, true)
                .0
                == Effect::Deny
            {
                return Err("blocked by permission deny rule".into());
            }
        }
        match decision {
            ApprovalDecision::Deny => Err("blocked by permission (user denied)".into()),
            ApprovalDecision::Always if shell && !has_prefix => self.save_exact_approval(
                tool,
                args.get("command")
                    .and_then(JsonValue::as_str)
                    .ok_or("missing command")?,
            ),
            ApprovalDecision::Always => Err("always requires an exact shell command".into()),
            ApprovalDecision::AllowOnce => Ok(()),
        }
    }
}

fn absolute(path: &str, cwd: &std::path::Path) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("file_path must be nonempty".into());
    }
    let normalized = std::path::PathBuf::from(
        oino_tools::canonical_path(path, &cwd.to_string_lossy()).map_err(|e| e.to_string())?,
    );
    // Resolve existing ancestors too, so a symlinked destination cannot evade a deny.
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            let mut resolved = real;
            for name in suffix.iter().rev() {
                resolved.push(name);
            }
            return Ok(resolved.to_string_lossy().into_owned());
        }
        if let Some(name) = ancestor.file_name() {
            suffix.push(name.to_owned());
        }
        let Some(parent) = ancestor.parent() else {
            return Ok(normalized.to_string_lossy().into_owned());
        };
        ancestor = parent;
    }
}
fn policy_subjects(
    tool: &str,
    args: &JsonObject,
    cwd: &std::path::Path,
) -> Result<Vec<String>, String> {
    match tool {
        "read" | "write" | "edit" => Ok(vec![absolute(
            args.get("file_path")
                .and_then(JsonValue::as_str)
                .ok_or("file_path is required")?,
            cwd,
        )?]),
        "apply_patch" => oino_tools::patch_paths(
            args.get("input")
                .and_then(JsonValue::as_str)
                .ok_or("patch input is required")?,
        )
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| absolute(&p, cwd))
        .collect(),
        "bash" | "powershell" => {
            let command = args
                .get("command")
                .and_then(JsonValue::as_str)
                .ok_or("command is required")?;
            Ok(vec![
                autonomy::canonical_command(command).unwrap_or_else(|_| command.to_owned()),
            ])
        }
        _ => Ok(vec![
            serde_json::to_string(args).map_err(|e| e.to_string())?,
        ]),
    }
}
