//! Server registry state (unipi `src/bridge/registry.ts`): lifecycle, status and the
//! deterministic registration plan. Network-free; the extension drives connections.

use serde::Serialize;

use crate::config::{MAX_SERVERS, Resolved};
use crate::translate::{ToolDef, compare_code_units};

/// unipi `ServerStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    /// Connecting / listing tools.
    Starting,
    /// Connected; tools registered.
    Running,
    /// Failed.
    Error,
    /// Stopped (or disabled).
    Stopped,
}

impl Status {
    /// Status word.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Error => "error",
            Self::Stopped => "stopped",
        }
    }

    /// unipi status icon.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Error => "✗",
            Self::Starting => "◐",
            Self::Stopped => "○",
        }
    }
}

/// unipi `ServerState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct State {
    /// Name.
    pub name: String,
    /// Status.
    pub status: Status,
    /// Process id (stdio).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Registered tools.
    pub tool_count: usize,
    /// Error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// ISO start time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

/// unipi `McpRegistryEntry` (client held by the extension).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// Resolved config.
    pub resolved: Resolved,
    /// State.
    pub state: State,
    /// Registered tool names (sorted).
    pub tool_names: Vec<String>,
    /// Translated tools (for re-registration / exposure).
    pub tools: Vec<ToolDef>,
}

/// The registry.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    /// unipi `startServers` preconditions: unique names, ≤ 20 servers after replacement.
    pub(crate) fn check_batch(&self, servers: &[Resolved]) -> Result<(), String> {
        let mut seen: Vec<&str> = Vec::new();
        for s in servers {
            if seen.contains(&s.name.as_str()) {
                return Err(format!(
                    "Duplicate MCP server name in startup batch: \"{}\"",
                    s.name
                ));
            }
            seen.push(&s.name);
        }
        let replacing = servers
            .iter()
            .filter(|s| self.get(&s.name).is_some())
            .count();
        if self.entries.len() - replacing + servers.len() > MAX_SERVERS {
            return Err(format!(
                "Maximum number of MCP servers ({MAX_SERVERS}) reached. Stop a server before starting a new one."
            ));
        }
        Ok(())
    }

    /// Adds (or replaces) an entry in `starting` state.
    pub(crate) fn begin(&mut self, resolved: &Resolved, now_iso: &str) {
        let entry = Entry {
            resolved: resolved.clone(),
            state: State {
                name: resolved.name.clone(),
                status: Status::Starting,
                pid: None,
                tool_count: 0,
                error: None,
                started_at: Some(now_iso.to_owned()),
            },
            tool_names: Vec::new(),
            tools: Vec::new(),
        };
        match self
            .entries
            .iter_mut()
            .find(|e| e.resolved.name == resolved.name)
        {
            Some(e) => *e = entry,
            None => self.entries.push(entry),
        }
    }

    /// Records a configured-but-disabled server (listed as stopped).
    pub(crate) fn disabled(&mut self, resolved: &Resolved) {
        if self.get(&resolved.name).is_none() {
            self.entries.push(Entry {
                resolved: resolved.clone(),
                state: State {
                    name: resolved.name.clone(),
                    status: Status::Stopped,
                    pid: None,
                    tool_count: 0,
                    error: None,
                    started_at: None,
                },
                tool_names: Vec::new(),
                tools: Vec::new(),
            });
        }
    }

    /// Marks a server failed.
    pub(crate) fn fail(&mut self, name: &str, error: &str) {
        if let Some(e) = self.get_mut(name) {
            e.state.status = Status::Error;
            e.state.tool_count = 0;
            e.state.error = Some(error.to_owned());
            e.tool_names.clear();
            e.tools.clear();
        }
    }

    /// unipi registration barrier: every prepared server's tools sorted by final name, with
    /// duplicates (against running servers too) rejected.
    pub(crate) fn plan(
        &self,
        prepared: &[(String, Vec<ToolDef>)],
    ) -> Result<Vec<(String, ToolDef)>, String> {
        let mut plan: Vec<(String, ToolDef)> = prepared
            .iter()
            .flat_map(|(server, tools)| tools.iter().map(|t| (server.clone(), t.clone())))
            .collect();
        plan.sort_by(|a, b| compare_code_units(&a.1.name, &b.1.name));
        let preparing: Vec<&str> = prepared.iter().map(|(s, _)| s.as_str()).collect();
        let existing: Vec<&String> = self
            .entries
            .iter()
            .filter(|e| {
                e.state.status == Status::Running && !preparing.contains(&e.resolved.name.as_str())
            })
            .flat_map(|e| e.tool_names.iter())
            .collect();
        let mut seen: Vec<&str> = existing.iter().map(|s| s.as_str()).collect();
        let mut dups: Vec<String> = Vec::new();
        for (_, t) in &plan {
            if seen.contains(&t.name.as_str()) {
                if !dups.contains(&t.name) {
                    dups.push(t.name.clone());
                }
            } else {
                seen.push(&t.name);
            }
        }
        if dups.is_empty() {
            Ok(plan)
        } else {
            dups.sort_by(|a, b| compare_code_units(a, b));
            Err(format!(
                "Duplicate final MCP tool name(s): {}",
                dups.join(", ")
            ))
        }
    }

    /// Marks a server running with its registered tools.
    pub(crate) fn running(&mut self, name: &str, pid: Option<u32>, mut tools: Vec<ToolDef>) {
        if let Some(e) = self.get_mut(name) {
            tools.sort_by(|a, b| compare_code_units(&a.name, &b.name));
            e.tool_names = tools.iter().map(|t| t.name.clone()).collect();
            e.state.status = Status::Running;
            e.state.pid = pid;
            e.state.tool_count = tools.len();
            e.state.error = None;
            e.tools = tools;
        }
    }

    /// unipi `stopServer` state change; returns the tool names to unregister.
    pub(crate) fn stop(&mut self, name: &str) -> Vec<String> {
        let Some(e) = self.get_mut(name) else {
            return Vec::new();
        };
        e.state.status = Status::Stopped;
        e.state.tool_count = 0;
        e.state.pid = None;
        e.tools.clear();
        std::mem::take(&mut e.tool_names)
    }

    /// Removes an entry entirely.
    pub(crate) fn remove(&mut self, name: &str) -> Vec<String> {
        let names = self.stop(name);
        self.entries.retain(|e| e.resolved.name != name);
        names
    }

    /// Every entry (insertion order).
    pub(crate) fn all(&self) -> &[Entry] {
        &self.entries
    }

    /// Running entries.
    pub(crate) fn active(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.state.status == Status::Running)
    }

    /// Failed entries.
    pub(crate) fn failed(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.state.status == Status::Error)
    }

    /// unipi `getTotalToolCount`.
    pub(crate) fn total_tools(&self) -> usize {
        self.active().map(|e| e.state.tool_count).sum()
    }

    /// Entry by name.
    pub(crate) fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.resolved.name == name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.resolved.name == name)
    }

    /// Server owning a final tool name.
    pub(crate) fn owner_of(&self, tool: &str) -> Option<(&Entry, &ToolDef)> {
        self.active()
            .find_map(|e| e.tools.iter().find(|t| t.name == tool).map(|t| (e, t)))
    }

    /// unipi `/unipi:mcp-status` text.
    pub(crate) fn status_text(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let mut lines = vec!["MCP Server Status:\n".to_owned()];
        for e in &self.entries {
            let s = &e.state;
            let info = match s.status {
                Status::Running if s.tool_count > 0 => format!(" ({} tools)", s.tool_count),
                Status::Error => s
                    .error
                    .as_ref()
                    .map(|err| format!(" — {err}"))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            lines.push(format!(
                "{} {} — {}{info}",
                s.status.icon(),
                s.name,
                s.status.as_str()
            ));
        }
        lines.push(format!(
            "\n---\n{} active, {} failed, {} total tools",
            self.active().count(),
            self.failed().count(),
            self.total_tools()
        ));
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::{Source, parse_entry};
    use crate::translate::translate;

    fn resolved(name: &str) -> Resolved {
        Resolved {
            name: name.into(),
            def: parse_entry(name, &json!({"command": name, "args": []}))
                .unwrap_or_else(|_| unreachable!()),
            source: Source::Global,
            enabled: true,
        }
    }

    fn tools(server: &str, names: &[&str]) -> Vec<ToolDef> {
        names
            .iter()
            .filter_map(|n| translate(server, &json!({"name": n, "description": n, "inputSchema": {"type": "object", "properties": {}}})))
            .collect()
    }

    #[test]
    fn registers_combined_tools_by_final_name() {
        let mut r = Registry::default();
        let servers = [resolved("zebra"), resolved("alpha")];
        assert!(r.check_batch(&servers).is_ok());
        for s in &servers {
            r.begin(s, "T");
        }
        let prepared = vec![
            ("zebra".to_owned(), tools("zebra", &["a", "z"])),
            ("alpha".to_owned(), tools("alpha", &["z", "b"])),
        ];
        let plan = r.plan(&prepared).unwrap_or_default();
        let names: Vec<&str> = plan.iter().map(|(_, t)| t.name.as_str()).collect();
        assert_eq!(names, ["alpha__b", "alpha__z", "zebra__a", "zebra__z"]);
        for (s, t) in prepared {
            r.running(&s, Some(1), t);
        }
        assert_eq!(
            r.get("alpha").map(|e| e.tool_names.clone()),
            Some(vec!["alpha__b".into(), "alpha__z".into()])
        );
        assert_eq!(r.total_tools(), 4);
        assert_eq!(
            r.owner_of("zebra__z")
                .map(|(e, t)| (e.resolved.name.as_str(), t.mcp_name.as_str())),
            Some(("zebra", "z"))
        );
    }

    #[test]
    fn single_server_sorted_by_code_units() {
        let r = Registry::default();
        let plan = r
            .plan(&[("server".into(), tools("server", &["z", "A", "a"]))])
            .unwrap_or_default();
        let names: Vec<&str> = plan.iter().map(|(_, t)| t.name.as_str()).collect();
        assert_eq!(names, ["server__A", "server__a", "server__z"]);
    }

    #[test]
    fn duplicate_final_names_fail_before_registration() {
        let r = Registry::default();
        let err = r
            .plan(&[
                ("a".into(), tools("a", &["b__same"])),
                ("a__b".into(), tools("a__b", &["same"])),
            ])
            .err()
            .unwrap_or_default();
        assert_eq!(err, "Duplicate final MCP tool name(s): a__b__same");
    }

    #[test]
    fn duplicates_against_running_servers() {
        let mut r = Registry::default();
        r.begin(&resolved("x"), "T");
        r.running("x", None, tools("x", &["t"]));
        assert!(
            r.plan(&[(
                "y".into(),
                vec![ToolDef {
                    name: "x__t".into(),
                    ..tools("y", &["t"])[0].clone()
                }]
            )])
            .is_err()
        );
        assert!(
            r.plan(&[("x".into(), tools("x", &["t"]))]).is_ok(),
            "a restarting server replaces its own tools"
        );
    }

    #[test]
    fn batch_limits() {
        let mut r = Registry::default();
        assert!(
            r.check_batch(&[resolved("a"), resolved("a")])
                .is_err_and(|e| e.contains("Duplicate MCP server name"))
        );
        let many: Vec<Resolved> = (0..=MAX_SERVERS)
            .map(|i| resolved(&format!("s{i}")))
            .collect();
        assert!(
            r.check_batch(&many)
                .is_err_and(|e| e.contains("Maximum number of MCP servers (20)"))
        );
        for s in &many[..MAX_SERVERS] {
            r.begin(s, "T");
        }
        assert!(
            r.check_batch(&many[..1]).is_ok(),
            "replacement does not count twice"
        );
    }

    #[test]
    fn errors_and_status_text() {
        let mut r = Registry::default();
        assert_eq!(r.status_text(), None);
        r.begin(&resolved("ok"), "T");
        r.running("ok", Some(5), tools("ok", &["a", "b"]));
        r.begin(&resolved("bad"), "T");
        r.fail("bad", "spawn failed");
        r.disabled(&resolved("off"));
        let text = r.status_text().unwrap_or_default();
        assert_eq!(
            text,
            "MCP Server Status:\n\n● ok — running (2 tools)\n✗ bad — error — spawn failed\n○ off — stopped\n\n---\n1 active, 1 failed, 2 total tools"
        );
        assert_eq!(r.stop("ok"), vec!["ok__a".to_owned(), "ok__b".to_owned()]);
        assert_eq!(r.get("ok").map(|e| e.state.status), Some(Status::Stopped));
        assert_eq!(r.remove("bad"), Vec::<String>::new());
        assert!(r.get("bad").is_none());
    }
}
