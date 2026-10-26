//! Server configuration (unipi `src/config/{schema,manager}.ts`, pi `mcp.json` rules).
//!
//! Files (Claude Desktop / pi `mcpServers` format):
//! - user: `/ext-global/mcp.json` (unipi `~/.unipi/config/mcp/mcp-config.json`), enabled states
//!   in `/ext-global/config.json` (unipi `config.json`, `{servers: {name: {enabled, addedAt}},
//!   sync}`);
//! - project: `/project/.oino/mcp.json` (+ `.oino/mcp-config.json` for unipi parity), only in a
//!   trusted project; a project entry wins (`project-override`), and an entry without
//!   `command`/`url`/`type` only overrides `enabled`/`exposure`/`toolExposure` (pi);
//! - ACP `session/new` servers from bus topic `oino:acp:mcp-servers`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// unipi `MCP_DEFAULTS.MAX_SERVERS`.
pub(crate) const MAX_SERVERS: usize = 20;
/// unipi `MCP_DEFAULTS.STARTUP_TIMEOUT_MS`.
pub(crate) const STARTUP_TIMEOUT_MS: u64 = 10_000;

/// Transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Transport {
    /// Child process over stdio.
    Stdio,
    /// Streamable HTTP.
    Http,
}

/// One server definition (unipi `McpServerDef` + pi fields).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerDef {
    /// Transport.
    pub transport: Transport,
    /// Executable (stdio).
    #[serde(default)]
    pub command: String,
    /// Arguments (stdio).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment (stdio).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Working directory (stdio; relative to the session cwd).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Endpoint (HTTP).
    #[serde(default)]
    pub url: String,
    /// Headers (HTTP).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Per-request timeout in ms.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Server exposure (`direct` | `deferred` | `hidden`; pi `codemode` = `deferred`).
    #[serde(default)]
    pub exposure: Option<String>,
    /// Per-tool exposure (exact names win, then the first matching `*` pattern).
    #[serde(default)]
    pub tool_exposure: Vec<(String, String)>,
    /// One-line description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Where a resolved server came from (unipi `ServerSource` + `acp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Source {
    /// User file.
    Global,
    /// Project file only.
    Project,
    /// Project entry replacing a user entry.
    ProjectOverride,
    /// ACP client session.
    Acp,
}

impl Source {
    /// Display label.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::ProjectOverride => "project-override",
            Self::Acp => "acp",
        }
    }
}

/// unipi `ResolvedServer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// Name.
    pub name: String,
    /// Definition.
    pub def: ServerDef,
    /// Source.
    pub source: Source,
    /// Enabled.
    pub enabled: bool,
}

/// A partial project entry: `(name, exposure, toolExposure)`.
pub(crate) type Partial = (String, Option<String>, Vec<(String, String)>);

/// One parsed config file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConfigFile {
    /// Full definitions.
    pub servers: Vec<(String, ServerDef)>,
    /// `enabled` from the entry itself (pi) or partial overrides.
    pub enabled: BTreeMap<String, bool>,
    /// Partial project entries (`enabled`/`exposure`/`toolExposure` only).
    pub partial: Vec<Partial>,
    /// Per-entry problems (entry skipped).
    pub errors: Vec<String>,
}

/// pi name rule: letters, digits, `_`, `-`.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn string_map(
    v: Option<&Value>,
    what: &str,
    name: &str,
    errors: &mut Vec<String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match v {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            for (k, val) in m {
                match val.as_str() {
                    Some(s) => {
                        out.insert(k.clone(), s.to_owned());
                    }
                    None => errors.push(format!("Server '{name}': {what}.{k} must be a string")),
                }
            }
        }
        Some(_) => errors.push(format!(
            "Server '{name}': '{what}' must be an object if present"
        )),
    }
    out
}

fn exposure_map(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Parses one `mcpServers` entry; `Err` lists why it is skipped (unipi `validateMcpConfig`
/// messages for stdio entries, pi rules for `type`/`url`/names).
pub(crate) fn parse_entry(name: &str, v: &Value) -> Result<ServerDef, Vec<String>> {
    let mut errors = Vec::new();
    let Some(o) = v.as_object() else {
        return Err(vec![format!("Server '{name}': must be a non-null object")]);
    };
    if !valid_name(name) {
        errors.push(format!(
            "Server '{name}': names may contain only letters, digits, '_' and '-'"
        ));
    }
    let ty = o.get("type").and_then(Value::as_str);
    let transport = match ty {
        Some("sse") => {
            return Err(vec![format!(
                "Server '{name}': the legacy SSE transport is not supported; use streamable HTTP (often at /mcp)"
            )]);
        }
        Some("http" | "streamable-http") => Transport::Http,
        Some("stdio") => Transport::Stdio,
        Some(other) => {
            return Err(vec![format!(
                "Server '{name}': 'type' must be stdio, http or streamable-http (got '{other}')"
            )]);
        }
        None if !o.contains_key("command") && o.contains_key("url") => Transport::Http,
        None => Transport::Stdio,
    };
    let timeout_ms = o
        .get("timeout")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
        .map(|s| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "positive seconds"
            )]
            let ms = (s * 1000.0) as u64;
            ms
        });
    let mut def = ServerDef {
        transport: transport.clone(),
        command: String::new(),
        args: Vec::new(),
        env: BTreeMap::new(),
        cwd: o.get("cwd").and_then(Value::as_str).map(ToOwned::to_owned),
        url: String::new(),
        headers: BTreeMap::new(),
        timeout_ms,
        exposure: o
            .get("exposure")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        tool_exposure: exposure_map(o.get("toolExposure")),
        description: o
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    };
    match transport {
        Transport::Stdio => {
            match o.get("command").and_then(Value::as_str) {
                Some(c) if !c.trim().is_empty() => c.clone_into(&mut def.command),
                _ => errors.push(format!(
                    "Server '{name}': 'command' must be a non-empty string"
                )),
            }
            match o.get("args") {
                None => {}
                Some(Value::Array(a)) => {
                    for (i, arg) in a.iter().enumerate() {
                        match arg.as_str() {
                            Some(s) => def.args.push(s.to_owned()),
                            None => {
                                errors.push(format!("Server '{name}': args[{i}] must be a string"));
                            }
                        }
                    }
                }
                Some(_) => errors.push(format!("Server '{name}': 'args' must be an array")),
            }
            def.env = string_map(o.get("env"), "env", name, &mut errors);
        }
        Transport::Http => {
            match o.get("url").and_then(Value::as_str) {
                Some(u) if u.starts_with("http://") || u.starts_with("https://") => {
                    u.clone_into(&mut def.url);
                }
                _ => errors.push(format!("Server '{name}': 'url' must be an http(s) URL")),
            }
            def.headers = string_map(o.get("headers"), "headers", name, &mut errors);
        }
    }
    if errors.is_empty() {
        Ok(def)
    } else {
        Err(errors)
    }
}

fn is_partial(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        !o.contains_key("command") && !o.contains_key("url") && !o.contains_key("type")
    })
}

/// Parses an `{mcpServers: {...}}` document (`project` allows partial overrides).
pub(crate) fn parse_file(doc: &Value, project: bool) -> ConfigFile {
    let mut out = ConfigFile::default();
    let Some(servers) = doc.get("mcpServers") else {
        if !doc.is_null() {
            out.errors
                .push("Missing or invalid 'mcpServers' field".to_owned());
        }
        return out;
    };
    let Some(map) = servers.as_object() else {
        out.errors
            .push("Missing or invalid 'mcpServers' field".to_owned());
        return out;
    };
    for (name, v) in map {
        if let Some(e) = v.get("enabled").and_then(Value::as_bool) {
            out.enabled.insert(name.clone(), e);
        }
        if project && is_partial(v) {
            out.partial.push((
                name.clone(),
                v.get("exposure")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                exposure_map(v.get("toolExposure")),
            ));
            continue;
        }
        match parse_entry(name, v) {
            Ok(def) => out.servers.push((name.clone(), def)),
            Err(errs) => out.errors.extend(errs),
        }
    }
    out
}

/// unipi `McpMetadata.servers` (`{name: {enabled, addedAt}}`).
pub(crate) fn meta_enabled(meta: &Value) -> BTreeMap<String, bool> {
    meta.get("servers")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    v.get("enabled")
                        .and_then(Value::as_bool)
                        .map(|e| (k.clone(), e))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// unipi `resolveServers` + pi partial overrides + ACP servers (ACP never replaces files).
pub(crate) fn resolve(
    global: &ConfigFile,
    global_meta: &BTreeMap<String, bool>,
    project: Option<&ConfigFile>,
    project_meta: &BTreeMap<String, bool>,
    acp: &[(String, ServerDef)],
) -> Vec<Resolved> {
    let mut merged: Vec<Resolved> = global
        .servers
        .iter()
        .map(|(name, def)| Resolved {
            name: name.clone(),
            def: def.clone(),
            source: Source::Global,
            enabled: global_meta
                .get(name)
                .or_else(|| global.enabled.get(name))
                .copied()
                .unwrap_or(true),
        })
        .collect();
    if let Some(p) = project {
        for (name, def) in &p.servers {
            let existing = merged.iter().position(|r| r.name == *name);
            let r = Resolved {
                name: name.clone(),
                def: def.clone(),
                source: if existing.is_some() {
                    Source::ProjectOverride
                } else {
                    Source::Project
                },
                enabled: p.enabled.get(name).copied().unwrap_or(true),
            };
            match existing {
                Some(i) => merged[i] = r,
                None => merged.push(r),
            }
        }
        for (name, exposure, tool_exposure) in &p.partial {
            if let Some(r) = merged.iter_mut().find(|r| r.name == *name) {
                if let Some(e) = p.enabled.get(name) {
                    r.enabled = *e;
                }
                if exposure.is_some() {
                    r.def.exposure.clone_from(exposure);
                }
                if !tool_exposure.is_empty() {
                    r.def.tool_exposure.clone_from(tool_exposure);
                }
            }
        }
    }
    for (name, enabled) in project_meta {
        if let Some(r) = merged.iter_mut().find(|r| r.name == *name) {
            r.enabled = *enabled;
        }
    }
    for (name, def) in acp {
        if !merged
            .iter()
            .any(|r| normalized(&r.name) == normalized(name))
        {
            merged.push(Resolved {
                name: name.clone(),
                def: def.clone(),
                source: Source::Acp,
                enabled: true,
            });
        }
    }
    merged
}

/// Names differing only in `-`/`_` are the same server (pi).
pub(crate) fn normalized(name: &str) -> String {
    name.replace('-', "_")
}

/// ACP `session/new` `mcpServers` → definitions (stdio `{name, command, args, env:[{name,
/// value}]}`, http `{type:"http", name, url, headers:[{name, value}]}`; sse rejected).
pub(crate) fn from_acp(payload: &Value) -> Vec<(String, ServerDef)> {
    let list = payload
        .get("servers")
        .or_else(|| payload.get("mcpServers"))
        .unwrap_or(payload);
    let Some(items) = list.as_array() else {
        return Vec::new();
    };
    let pairs = |v: Option<&Value>| -> Map<String, Value> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some((
                            p.get("name")?.as_str()?.to_owned(),
                            json!(p.get("value")?.as_str()?),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_owned();
            let mut entry = s.as_object()?.clone();
            entry.remove("name");
            if entry.contains_key("env") {
                entry.insert("env".into(), Value::Object(pairs(s.get("env"))));
            }
            if entry.contains_key("headers") {
                entry.insert("headers".into(), Value::Object(pairs(s.get("headers"))));
            }
            parse_entry(&name, &Value::Object(entry))
                .ok()
                .map(|d| (name, d))
        })
        .collect()
}

/// Effective exposure of `tool` (pi: exact names win, then the first matching pattern).
pub(crate) fn tool_exposure(def: &ServerDef, tool: &str) -> String {
    if let Some((_, e)) = def.tool_exposure.iter().find(|(k, _)| k == tool) {
        return e.clone();
    }
    if let Some((_, e)) = def
        .tool_exposure
        .iter()
        .find(|(k, _)| k.contains('*') && glob_match(k, tool))
    {
        return e.clone();
    }
    def.exposure.clone().unwrap_or_else(|| "direct".to_owned())
}

/// `*` wildcard match.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    rest.is_empty()
}

/// Expands `${VAR}` in a value with `lookup`; a whole-value `!command` is returned as
/// `Err(command)` for the caller to run. Unresolved variables are left as written.
pub(crate) fn expand(
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if let Some(cmd) = value.strip_prefix('!') {
        return Err(cmd.trim().to_owned());
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            if let Some(v) = lookup(name) {
                out.push_str(&v);
            } else {
                out.push_str("${");
                out.push_str(name);
                out.push('}');
            }
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// `~/x` → `<home>/x`.
pub(crate) fn expand_home(value: &str, home: Option<&str>) -> String {
    match (value.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => format!("{}/{rest}", h.trim_end_matches('/')),
        _ => value.to_owned(),
    }
}

/// An `mcpServers` entry as JSON (for the editor and for saving).
pub(crate) fn entry_json(def: &ServerDef) -> Value {
    let mut o = Map::new();
    match def.transport {
        Transport::Stdio => {
            o.insert("command".into(), json!(def.command));
            o.insert("args".into(), json!(def.args));
            if !def.env.is_empty() {
                o.insert("env".into(), json!(def.env));
            }
            if let Some(c) = &def.cwd {
                o.insert("cwd".into(), json!(c));
            }
        }
        Transport::Http => {
            o.insert("url".into(), json!(def.url));
            if !def.headers.is_empty() {
                o.insert("headers".into(), json!(def.headers));
            }
        }
    }
    if let Some(t) = def.timeout_ms {
        o.insert("timeout".into(), json!(t / 1000));
    }
    if let Some(e) = &def.exposure {
        o.insert("exposure".into(), json!(e));
    }
    if !def.tool_exposure.is_empty() {
        let m: Map<String, Value> = def
            .tool_exposure
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        o.insert("toolExposure".into(), Value::Object(m));
    }
    if let Some(d) = &def.description {
        o.insert("description".into(), json!(d));
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(cmd: &str) -> Value {
        json!({"command": cmd, "args": []})
    }

    #[test]
    fn validation_like_unipi() {
        let doc = json!({"mcpServers": {
            "ok": {"command": "npx", "args": ["-y", "x"], "env": {"A": "1"}},
            "nocmd": {"args": []},
            "badargs": {"command": "x", "args": [1]},
            "badenv": {"command": "x", "args": [], "env": {"A": 1}},
            "bad name": {"command": "x", "args": []},
            "sse": {"type": "sse", "url": "https://x/sse"},
            "web": {"url": "https://example.com/mcp", "headers": {"Authorization": "Bearer ${T}"}, "timeout": 30},
        }});
        let f = parse_file(&doc, false);
        let names: Vec<&str> = f.servers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["ok", "web"]);
        assert!(
            f.errors
                .contains(&"Server 'nocmd': 'command' must be a non-empty string".to_owned())
        );
        assert!(
            f.errors
                .contains(&"Server 'badargs': args[0] must be a string".to_owned())
        );
        assert!(
            f.errors
                .contains(&"Server 'badenv': env.A must be a string".to_owned())
        );
        assert!(f.errors.iter().any(|e| e.contains("'bad name'")));
        assert!(f.errors.iter().any(|e| e.contains("SSE")));
        let web = &f.servers[1].1;
        assert_eq!(web.transport, Transport::Http);
        assert_eq!(web.timeout_ms, Some(30_000));
        assert_eq!(
            parse_file(&json!({"x": 1}), false).errors,
            ["Missing or invalid 'mcpServers' field"]
        );
        assert!(parse_file(&Value::Null, false).errors.is_empty());
    }

    #[test]
    fn merge_rules_like_unipi() {
        let global = parse_file(
            &json!({"mcpServers": {"a": stdio("ga"), "b": stdio("gb"), "c": stdio("gc")}}),
            false,
        );
        let project = parse_file(
            &json!({"mcpServers": {"b": stdio("pb"), "d": stdio("pd"), "c": {"enabled": false, "exposure": "deferred"}}}),
            true,
        );
        let gmeta = BTreeMap::from([("a".to_owned(), false)]);
        let pmeta = BTreeMap::from([("d".to_owned(), false)]);
        let acp = vec![(
            "e".to_owned(),
            parse_entry("e", &stdio("ae")).unwrap_or_else(|_| unreachable!()),
        )];
        let r = resolve(&global, &gmeta, Some(&project), &pmeta, &acp);
        let view: Vec<(&str, &str, bool, &str)> = r
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.source.as_str(),
                    r.enabled,
                    r.def.command.as_str(),
                )
            })
            .collect();
        assert_eq!(
            view,
            [
                ("a", "global", false, "ga"),
                ("b", "project-override", true, "pb"),
                ("c", "global", false, "gc"),
                ("d", "project", false, "pd"),
                ("e", "acp", true, "ae"),
            ]
        );
        assert_eq!(r[2].def.exposure.as_deref(), Some("deferred"));
        let dup = vec![("a".to_owned(), r[0].def.clone())];
        assert_eq!(
            resolve(&global, &BTreeMap::new(), None, &BTreeMap::new(), &dup).len(),
            3,
            "files win over ACP"
        );
    }

    #[test]
    fn acp_servers() {
        let p = json!({"sessionId": "s", "servers": [
            {"name": "fs", "command": "npx", "args": ["x"], "env": [{"name": "A", "value": "1"}]},
            {"type": "http", "name": "docs", "url": "https://d/mcp", "headers": [{"name": "X", "value": "y"}]},
            {"type": "sse", "name": "old", "url": "https://d/sse", "headers": []},
        ]});
        let s = from_acp(&p);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].1.env.get("A").map(String::as_str), Some("1"));
        assert_eq!(s[1].1.headers.get("X").map(String::as_str), Some("y"));
        assert!(from_acp(&json!("junk")).is_empty());
    }

    #[test]
    fn exposure_patterns() {
        let def = parse_entry(
            "gh",
            &json!({"url": "https://x/mcp", "exposure": "deferred", "toolExposure": {"get_*": "hidden", "search_code": "direct", "*_x": "direct"}}),
        )
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(tool_exposure(&def, "search_code"), "direct");
        assert_eq!(tool_exposure(&def, "get_issue"), "hidden");
        assert_eq!(tool_exposure(&def, "list"), "deferred");
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b", "aXXc"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn variable_expansion() {
        let lookup = |n: &str| (n == "TOKEN").then(|| "abc".to_owned());
        assert_eq!(
            expand("Bearer ${TOKEN}", &lookup),
            Ok("Bearer abc".to_owned())
        );
        assert_eq!(
            expand("${MISSING}-x", &lookup),
            Ok("${MISSING}-x".to_owned())
        );
        assert_eq!(
            expand("!gh auth token", &lookup),
            Err("gh auth token".to_owned())
        );
        assert_eq!(expand("broken ${x", &lookup), Ok("broken ${x".to_owned()));
        assert_eq!(expand_home("~/bin/x", Some("/home/u/")), "/home/u/bin/x");
        assert_eq!(expand_home("~/bin/x", None), "~/bin/x");
    }

    #[test]
    fn entry_round_trip() {
        let v = json!({"command": "npx", "args": ["-y", "x"], "env": {"K": "v"}, "timeout": 30, "description": "d"});
        let d = parse_entry("x", &v).unwrap_or_else(|_| unreachable!());
        assert_eq!(entry_json(&d), v);
        assert_eq!(normalized("a-b"), normalized("a_b"));
    }
}
