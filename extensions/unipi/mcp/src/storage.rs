//! MCP file discovery and canonical writes. Compatibility files are strictly read-only.
use crate::config::{self, ConfigFile, Resolved, ServerDef};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const PROJECT_FILES: [&str; 4] = [
    ".oino/mcp.json",
    ".pi/mcp.json",
    ".unipi/config/mcp/mcp-config.json",
    ".mcp.json",
];
const META: &str = ".unipi/config/mcp/config.json";

pub(crate) fn read(path: &Path, errors: &mut Vec<String>) -> Option<Value> {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(v) => Some(v),
            Err(e) => {
                errors.push(format!(
                    "{}: parse error at line {}",
                    path.display(),
                    e.line()
                ));
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            errors.push(format!("{}: unreadable", path.display()));
            None
        }
    }
}

fn file(path: &Path, project: bool, errors: &mut Vec<String>) -> ConfigFile {
    let Some(doc) = read(path, errors) else {
        return ConfigFile::default();
    };
    let mut f = config::parse_file(&doc, project);
    errors.extend(f.errors.iter().map(|e| format!("{}: {e}", path.display())));
    for name in f
        .servers
        .iter()
        .map(|(n, _)| n)
        .chain(f.partial.iter().map(|(n, _, _)| n))
    {
        f.paths.insert(name.clone(), path.display().to_string());
    }
    f
}

pub(crate) fn load(
    global: &Path,
    project: &Path,
    trusted: bool,
    acp: &[(String, ServerDef)],
) -> (Vec<Resolved>, Vec<String>) {
    let mut errors = Vec::new();
    let global = file(global, false, &mut errors);
    let mut p = ConfigFile::default();
    let mut meta = BTreeMap::new();
    if trusted {
        if let Some(doc) = read(&project.join(META), &mut errors) {
            meta = config::meta_enabled(&doc);
        }
        // Lowest first; a full OR partial entry replaces the lower file's whole entry.
        for relative in PROJECT_FILES.iter().rev() {
            let mut f = file(&project.join(relative), true, &mut errors);
            if *relative == PROJECT_FILES[2] {
                for (name, enabled) in &meta {
                    f.enabled.entry(name.clone()).or_insert(*enabled);
                }
            }
            // A full higher-precedence definition replaces lower metadata too.
            if *relative != PROJECT_FILES[2] {
                for (name, _) in &f.servers {
                    meta.remove(name);
                }
            }
            // Higher precedence explicit flags win over legacy project metadata.
            for (name, enabled) in &f.enabled {
                meta.insert(name.clone(), *enabled);
            }
            config::absorb(&mut p, f);
        }
    } else {
        // Trust is reported once by the page header. Do not invent warnings for
        // absent candidates (or expose the host's internal scratch folder).
    }
    (
        config::resolve(&global, &BTreeMap::new(), trusted.then_some(&p), &meta, acp),
        errors,
    )
}

/// Merge legacy state only once. Never overwrite a canonical entry. Rename only after
/// the canonical write succeeds; retrying after an interrupted rename is idempotent.
pub(crate) fn migrate(
    global: &Path,
    legacy: &Path,
    write: &dyn Fn(&Path, &Value) -> Result<(), String>,
) -> Result<(), String> {
    let old = legacy.join("mcp.json");
    let metadata = legacy.join("config.json");
    if !old.exists() && !metadata.exists() {
        return Ok(());
    }
    let mut errors = Vec::new();
    let old_doc = read(&old, &mut errors);
    let meta = read(&metadata, &mut errors)
        .map(|v| config::meta_enabled(&v))
        .unwrap_or_default();
    let mut doc = read(global, &mut errors).unwrap_or_else(|| json!({"mcpServers": {}}));
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let Some(entries) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return Err("Global MCP file: invalid mcpServers".into());
    };
    if let Some(old_doc) = old_doc {
        let Some(old_entries) = old_doc.get("mcpServers").and_then(Value::as_object) else {
            return Err("Legacy MCP file: invalid mcpServers".into());
        };
        for (name, entry) in old_entries {
            if !entries.contains_key(name) {
                let mut entry = entry.clone();
                if let Some(enabled) = meta.get(name) {
                    entry["enabled"] = json!(enabled);
                }
                entries.insert(name.clone(), entry);
            }
        }
    }
    write(global, &doc)?;
    for path in [old, metadata] {
        if path.exists() {
            std::fs::rename(&path, path.with_extension("json.migrated"))
                .map_err(|_| "Could not rename migrated MCP state".to_owned())?;
        }
    }
    Ok(())
}

pub(crate) fn writable(
    global: &Path,
    project: &Path,
    scope: &str,
    trusted: bool,
) -> Result<PathBuf, String> {
    match scope {
        "global" => Ok(global.to_path_buf()),
        "project" if trusted => Ok(project.join(PROJECT_FILES[0])),
        "project" => Err("Not trusted — project servers aren't loaded".into()),
        _ => Err("scope must be global or project".into()),
    }
}

/// Refuse to overwrite broken documents (including unreadable ones).
pub(crate) fn edit(
    path: &Path,
    change: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    let mut doc = read(path, &mut errors).unwrap_or_else(|| json!({"mcpServers": {}}));
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let entries = doc
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .ok_or("Invalid mcpServers")?;
    change(entries)?;
    Ok(doc)
}

fn upper_var(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_uppercase()
}

/// Store key / `${…}` name of a server's header or env secret: `<SERVER>_<KEY>`. A key that already starts with the
/// server's name is not prefixed again (`refero` + `REFERO_AUTH` → `REFERO_AUTH`). Same rule as `secretVar` in the app.
pub(crate) fn secret_key(server: &str, key: &str) -> String {
    let (s, k) = (upper_var(server), upper_var(key));
    if !s.is_empty() && (k == s || k.starts_with(&format!("{s}_"))) {
        k
    } else {
        upper_var(&format!("{s}_{k}"))
    }
}

/// The name `secret_key` produced before the server prefix was de-duplicated.
fn legacy_secret_key(server: &str, key: &str) -> String {
    upper_var(&format!("{server}_{key}"))
}

/// One-time rename of `${[prefix]REFERO_REFERO_AUTH}` references to `${[prefix]REFERO_AUTH}` (N8). The value is copied
/// to the new name when that is not set yet; the old stored value stays (other files may still reference it).
/// Returns whether the entry changed. Idempotent.
pub(crate) fn dedupe_secret_names(
    server: &str,
    entry: &mut Value,
    prefix: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    store: &dyn Fn(&str, &str) -> Result<(), String>,
) -> Result<bool, String> {
    let mut changed = false;
    for field in ["env", "headers"] {
        let Some(map) = entry.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for (key, value) in map {
            let (old, new) = (legacy_secret_key(server, key), secret_key(server, key));
            if old == new || value.as_str() != Some(&format!("${{{prefix}{old}}}")) {
                continue;
            }
            let (old_full, new_full) = (format!("{prefix}{old}"), format!("{prefix}{new}"));
            if lookup(&new_full).is_none() {
                if let Some(secret) = lookup(&old_full) {
                    store(&new_full, &secret)?;
                } else {
                    continue;
                }
            }
            *value = json!(format!("${{{new_full}}}"));
            changed = true;
        }
    }
    Ok(changed)
}

pub(crate) fn bearer_looking(value: &str) -> bool {
    value.trim().to_ascii_lowercase().starts_with("bearer ")
}

/// Stable project identity from the host preopen, never the guest `/project` alias.
pub(crate) fn project_secret_prefix(project: &str) -> String {
    let hash = project
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("PROJECT_{hash:016X}_")
}

/// Copy legacy values before rewriting references. Global and other project keys
/// remain untouched. Re-running this migration never overwrites a scoped value.
pub(crate) fn scope_secrets(
    entry: &mut Value,
    prefix: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    store: &dyn Fn(&str, &str) -> Result<(), String>,
) -> Result<bool, String> {
    let mut changed = false;
    match entry {
        Value::String(text) => {
            let original = text.clone();
            let mut rest = original.as_str();
            while let Some(start) = rest.find("${") {
                let after = &rest[start + 2..];
                let Some(end) = after.find('}') else {
                    break;
                };
                let key = &after[..end];
                if !key.starts_with("PROJECT_")
                    && let Some(value) = lookup(key)
                {
                    let scoped = format!("{prefix}{key}");
                    if lookup(&scoped).is_none() {
                        store(&scoped, &value)?;
                    }
                    *text = text.replace(&format!("${{{key}}}"), &format!("${{{scoped}}}"));
                    changed = true;
                }
                rest = &after[end + 1..];
            }
        }
        Value::Array(items) => {
            for item in items {
                changed |= scope_secrets(item, prefix, lookup, store)?;
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                changed |= scope_secrets(value, prefix, lookup, store)?;
            }
        }
        _ => {}
    }
    Ok(changed)
}

pub(crate) fn secret_looking(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "token",
        "key",
        "secret",
        "auth",
        "password",
        "pat",
        "credential",
    ]
    .iter()
    .any(|p| key.contains(p))
}

/// Values selected by the form use secrets[server][key] (key is an env/header name).
/// Literal project credentials are also extracted automatically.
pub(crate) fn protect(
    server: &str,
    entry: &mut Value,
    supplied: Option<&Value>,
    project: bool,
    store: &dyn Fn(&str, &str) -> Result<(), String>,
) -> Result<usize, String> {
    let mut moved = 0;
    for field in ["env", "headers"] {
        if let Some(map) = entry.get_mut(field).and_then(Value::as_object_mut) {
            for (key, value) in map {
                let explicit = supplied.and_then(|s| s.get(key)).and_then(Value::as_str);
                let literal = value
                    .as_str()
                    .filter(|s| !s.contains("${") && !s.starts_with('!') && !s.is_empty());
                if let Some(secret) = explicit.or_else(|| {
                    (project && (secret_looking(key) || literal.is_some_and(bearer_looking)))
                        .then_some(literal)
                        .flatten()
                }) {
                    let name = secret_key(server, key);
                    store(&name, secret)?;
                    *value = json!(format!("${{{name}}}"));
                    moved += 1;
                }
            }
        }
    }
    Ok(moved)
}

/// No arbitrary host environment fallback: only the extension's secrets namespace.
pub(crate) fn expand(
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut missing = None;
    let out = config::expand(value, lookup)?;
    if let Some(start) = out.find("${") {
        missing = Some(
            out[start + 2..]
                .split('}')
                .next()
                .unwrap_or("variable")
                .to_owned(),
        );
    }
    match missing {
        Some(name) => Err(format!("{name} is not set — add it in Settings › MCP")),
        None => Ok(out),
    }
}

pub(crate) fn redact_entry(entry: &mut Value) {
    for field in ["env", "headers"] {
        if let Some(map) = entry.get_mut(field).and_then(Value::as_object_mut) {
            for (key, value) in map {
                if (secret_looking(key) || value.as_str().is_some_and(bearer_looking))
                    && !value
                        .as_str()
                        .is_some_and(|s| s.starts_with("${") && s.ends_with('}'))
                {
                    *value = json!("••••");
                }
            }
        }
    }
}

pub(crate) fn redact(text: &str, values: &[String]) -> String {
    let mut out = text.to_owned();
    let mut values: Vec<_> = values.iter().filter(|s| !s.is_empty()).collect();
    values.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for value in values {
        out = out.replace(value, "[REDACTED]");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
        static ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = PathBuf::from(std::env::var("HOME")?)
            .join(".cache/oino-work/mcp-wi2/unit")
            .join(format!(
                "{}-{}",
                std::process::id(),
                ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path)?;
        Ok(Fixture(path))
    }
    fn put(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(path.parent().ok_or("fixture parent")?)?;
        std::fs::write(path, serde_json::to_vec(value)?)?;
        Ok(())
    }
    #[test]
    fn four_files_precedence_sources_and_partial_global() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = fixture()?;
        let global = dir.path().join("global.json");
        put(
            &global,
            &json!({"mcpServers": {"shared": {"command": "global"}, "inherited": {"command": "global"}}}),
        )?;
        for (i, file) in PROJECT_FILES.iter().enumerate().rev() {
            put(
                &dir.path().join(file),
                &json!({"mcpServers": {"shared": {"command": format!("level{i}")}, format!("only{i}"): {"command": "x"}}}),
            )?;
        }
        let (r, e) = load(&global, dir.path(), true, &[]);
        assert!(e.is_empty());
        assert_eq!(r.len(), 6);
        assert_eq!(
            r.iter()
                .find(|r| r.name == "shared")
                .ok_or("expected fixture value")?
                .def
                .command,
            "level0"
        );
        for (i, file) in PROJECT_FILES.iter().enumerate() {
            assert!(
                r.iter()
                    .find(|r| r.name == format!("only{i}"))
                    .ok_or("expected fixture entry")?
                    .source_path
                    .as_ref()
                    .ok_or("expected source path")?
                    .ends_with(*file)
            );
        }
        put(
            &dir.path().join(PROJECT_FILES[0]),
            &json!({"mcpServers": {"shared": {"enabled": false, "exposure": "hidden"}}}),
        )?;
        let (r, _) = load(&global, dir.path(), true, &[]);
        let shared = r
            .iter()
            .find(|r| r.name == "shared")
            .ok_or("expected fixture value")?;
        assert_eq!(
            shared.def.command, "global",
            "partial inherits global, not lower project full entry"
        );
        assert!(!shared.enabled);
        assert_eq!(shared.source, config::Source::ProjectOverride);
        assert_eq!(shared.def.exposure.as_deref(), Some("hidden"));
        let (r, e) = load(&global, dir.path(), false, &[]);
        assert_eq!(r.len(), 2);
        assert!(e.is_empty(), "trust is shown once in the project header");
        assert!(e.iter().all(|e| e.ends_with("not trusted")));
        Ok(())
    }
    #[test]
    fn metadata_project_only_and_errors_visible() -> Result<(), Box<dyn std::error::Error>> {
        let dir = fixture()?;
        let global = dir.path().join("global.json");
        put(
            &global,
            &json!({"mcpServers": {"x": {"command": "g", "enabled": false}}}),
        )?;
        put(
            &dir.path().join(PROJECT_FILES[1]),
            &json!({"mcpServers": {"x": {"command": "p"}}}),
        )?;
        let (r, _) = load(&global, dir.path(), true, &[]);
        assert!(
            r[0].enabled,
            "global enabled does not leak into project replacement"
        );
        put(
            &dir.path().join(META),
            &json!({"servers": {"x": {"enabled": false}}}),
        )?;
        let (r, _) = load(&global, dir.path(), true, &[]);
        assert!(
            r[0].enabled,
            "full pi definition beats lower legacy metadata"
        );
        put(
            &dir.path().join(META),
            &json!({"servers": {"other": {"enabled": false}}}),
        )?;
        std::fs::create_dir_all(dir.path().join(".oino")).map_err(|error| format!("{error:?}"))?;
        std::fs::write(dir.path().join(PROJECT_FILES[0]), "{\n broken\n}")
            .map_err(|error| format!("{error:?}"))?;
        let (r, e) = load(&global, dir.path(), true, &[]);
        assert_eq!(r[0].def.command, "p");
        assert!(e.iter().any(|e| e.ends_with("parse error at line 2")));
        std::fs::create_dir(dir.path().join(PROJECT_FILES[3]))
            .map_err(|error| format!("{error:?}"))?;
        assert!(
            load(&global, dir.path(), true, &[])
                .1
                .iter()
                .any(|e| e.ends_with("unreadable"))
        );
        Ok(())
    }
    #[test]
    fn migration_is_idempotent_and_preserves_canonical() -> Result<(), Box<dyn std::error::Error>> {
        let dir = fixture()?;
        let global = dir.path().join("new/mcp.json");
        let old = dir.path().join("old");
        put(&global, &json!({"mcpServers": {"x": {"command": "new"}}}))?;
        put(
            &old.join("mcp.json"),
            &json!({"mcpServers": {"x": {"command": "old"}, "y": {"command": "y"}}}),
        )?;
        put(
            &old.join("config.json"),
            &json!({"servers": {"y": {"enabled": false}}}),
        )?;
        let write = |p: &Path, v: &Value| {
            put(p, v).map_err(|error| error.to_string())?;
            Ok(())
        };
        migrate(&global, &old, &write).map_err(|error| format!("{error:?}"))?;
        migrate(&global, &old, &write).map_err(|error| format!("{error:?}"))?;
        let doc = read(&global, &mut Vec::new()).ok_or("expected fixture document")?;
        assert_eq!(doc["mcpServers"]["x"]["command"], "new");
        assert_eq!(doc["mcpServers"]["y"]["enabled"], false);
        assert!(old.join("mcp.json.migrated").exists());
        assert!(old.join("config.json.migrated").exists());
        Ok(())
    }
    #[test]
    fn project_tokens_do_not_overwrite_each_other_or_global()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = std::cell::RefCell::new(BTreeMap::from([(
            "LINEAR_AUTHORIZATION".into(),
            "old-token".into(),
        )]));
        let lookup = |k: &str| store.borrow().get(k).cloned();
        let write = |k: &str, v: &str| {
            store.borrow_mut().insert(k.to_owned(), v.to_owned());
            Ok(())
        };
        let mut a = json!({"headers":{"Authorization":"${LINEAR_AUTHORIZATION}"}});
        let mut b = a.clone();
        let pa = project_secret_prefix("/projects/a");
        let pb = project_secret_prefix("/projects/b");
        assert_ne!(pa, pb);
        assert!(scope_secrets(&mut a, &pa, &lookup, &write).map_err(|error| format!("{error:?}"))?);
        assert!(scope_secrets(&mut b, &pb, &lookup, &write).map_err(|error| format!("{error:?}"))?);
        write(&format!("{pb}LINEAR_AUTHORIZATION"), "b-token")
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            lookup(&format!("{pa}LINEAR_AUTHORIZATION")).as_deref(),
            Some("old-token")
        );
        assert_eq!(lookup("LINEAR_AUTHORIZATION").as_deref(), Some("old-token"));
        assert!(
            !scope_secrets(&mut b, &pb, &lookup, &write).map_err(|error| format!("{error:?}"))?
        );
        assert!(b.to_string().contains(&pb));
        Ok(())
    }

    #[test]
    fn secret_names_do_not_repeat_the_server() {
        assert_eq!(secret_key("refero", "REFERO_AUTH"), "REFERO_AUTH");
        assert_eq!(secret_key("refero", "AUTH"), "REFERO_AUTH");
        assert_eq!(
            secret_key("linear", "Authorization"),
            "LINEAR_AUTHORIZATION"
        );
        assert_eq!(secret_key("my-server", "x-api-key"), "MY_SERVER_X_API_KEY");
        // A different server whose name merely starts the same is not a repeat.
        assert_eq!(secret_key("ref", "REFERO_AUTH"), "REF_REFERO_AUTH");
        assert_eq!(secret_key("refero", "REFEROX"), "REFERO_REFEROX");
    }

    #[test]
    fn duplicated_secret_names_are_renamed_once_and_keep_the_value()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = std::cell::RefCell::new(BTreeMap::from([(
            "PROJECT_AB_REFERO_REFERO_AUTH".to_owned(),
            "tok".to_owned(),
        )]));
        let lookup = |k: &str| store.borrow().get(k).cloned();
        let write = |k: &str, v: &str| {
            store.borrow_mut().insert(k.to_owned(), v.to_owned());
            Ok(())
        };
        let mut entry = json!({"command": "npx", "env": {"REFERO_AUTH": "${PROJECT_AB_REFERO_REFERO_AUTH}", "OTHER": "${PROJECT_AB_OTHER}"}});
        assert!(
            dedupe_secret_names("refero", &mut entry, "PROJECT_AB_", &lookup, &write)
                .map_err(|error| format!("{error:?}"))?
        );
        assert_eq!(entry["env"]["REFERO_AUTH"], "${PROJECT_AB_REFERO_AUTH}");
        assert_eq!(entry["env"]["OTHER"], "${PROJECT_AB_OTHER}");
        assert_eq!(lookup("PROJECT_AB_REFERO_AUTH").as_deref(), Some("tok"));
        // The old value stays; a second run changes nothing.
        assert_eq!(
            lookup("PROJECT_AB_REFERO_REFERO_AUTH").as_deref(),
            Some("tok")
        );
        assert!(
            !dedupe_secret_names("refero", &mut entry, "PROJECT_AB_", &lookup, &write)
                .map_err(|error| format!("{error:?}"))?
        );
        // A reference whose value is gone is left alone (never points at nothing new).
        let mut broken = json!({"env": {"REFERO_AUTH": "${PROJECT_ZZ_REFERO_REFERO_AUTH}"}});
        assert!(
            !dedupe_secret_names("refero", &mut broken, "PROJECT_ZZ_", &lookup, &write)
                .map_err(|error| format!("{error:?}"))?
        );
        // Global files use no prefix.
        let mut global =
            json!({"headers": {"LINEAR_AUTHORIZATION": "${LINEAR_LINEAR_AUTHORIZATION}"}});
        store
            .borrow_mut()
            .insert("LINEAR_LINEAR_AUTHORIZATION".into(), "g".into());
        assert!(
            dedupe_secret_names("linear", &mut global, "", &lookup, &write)
                .map_err(|error| format!("{error:?}"))?
        );
        assert_eq!(
            global["headers"]["LINEAR_AUTHORIZATION"],
            "${LINEAR_AUTHORIZATION}"
        );
        Ok(())
    }

    #[test]
    fn auth_password_pat_credential_and_bearer_are_masked() {
        for key in ["REFERO_AUTH", "DB_PASSWORD", "GH_PAT", "APP_CREDENTIAL"] {
            assert!(secret_looking(key));
        }
        let mut entry = json!({"env":{"REFERO_AUTH":"token", "CUSTOM":"Bearer value"}});
        redact_entry(&mut entry);
        assert_eq!(entry["env"]["REFERO_AUTH"], "••••");
        assert_eq!(entry["env"]["CUSTOM"], "••••");
    }

    #[test]
    fn secrets_roundtrip_redaction_and_unresolved() -> Result<(), Box<dyn std::error::Error>> {
        let store = std::cell::RefCell::new(BTreeMap::new());
        let mut entry = json!({"url": "https://example/mcp", "headers": {"Authorization": "Bearer fixture-secret", "X-Custom": "plain"}});
        assert_eq!(
            protect(
                "linear",
                &mut entry,
                Some(&json!({"X-Custom": "custom-secret"})),
                true,
                &|k, v| {
                    store.borrow_mut().insert(k.to_owned(), v.to_owned());
                    Ok(())
                }
            )
            .map_err(|error| format!("{error:?}"))?,
            2
        );
        assert_eq!(entry["headers"]["Authorization"], "${LINEAR_AUTHORIZATION}");
        assert!(!entry.to_string().contains("fixture-secret"));
        assert_eq!(
            expand(
                entry["headers"]["Authorization"]
                    .as_str()
                    .ok_or("expected fixture value")?,
                &|k| { store.borrow().get(k).cloned() }
            )
            .map_err(|error| format!("{error:?}"))?,
            "Bearer fixture-secret"
        );
        assert_eq!(
            expand("Bearer ${LINEAR_KEY}", &|_| None),
            Err("LINEAR_KEY is not set — add it in Settings › MCP".into())
        );
        assert_eq!(
            redact(
                "bad Bearer fixture-secret",
                &["fixture-secret".into(), "Bearer fixture-secret".into()]
            ),
            "bad [REDACTED]"
        );
        redact_entry(&mut entry);
        assert_eq!(entry["headers"]["Authorization"], "${LINEAR_AUTHORIZATION}");
        Ok(())
    }
    #[test]
    fn discovery_measurement_50_runs() -> Result<(), Box<dyn std::error::Error>> {
        let dir = fixture()?;
        let global = dir.path().join("global.json");
        put(
            &global,
            &json!({"mcpServers": {"global": {"command": "fixture", "enabled":false}}}),
        )?;
        for (i, path) in PROJECT_FILES.iter().enumerate() {
            put(
                &dir.path().join(path),
                &json!({"mcpServers": {format!("fixture{i}"): {"command":"fixture", "enabled":false}}}),
            )?;
        }
        let mut times = Vec::new();
        for _ in 0..50 {
            let start = std::time::Instant::now();
            let (r, errors) = load(&global, dir.path(), true, &[]);
            assert_eq!(r.len(), 5);
            assert!(errors.is_empty());
            times.push(start.elapsed().as_micros());
        }
        times.sort_unstable();
        println!(
            "WI2 discovery 5 files n=50 p50={}us p95={}us max={}us",
            times[25], times[47], times[49]
        );
        Ok(())
    }

    #[test]
    fn writes_only_canonical_and_never_clobber_bad_json() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = fixture()?;
        let global = dir.path().join("global.json");
        assert_eq!(
            writable(&global, dir.path(), "project", true).map_err(|error| format!("{error:?}"))?,
            dir.path().join(PROJECT_FILES[0])
        );
        assert!(writable(&global, dir.path(), "project", false).is_err());
        std::fs::write(&global, "{bad").map_err(|error| format!("{error:?}"))?;
        assert!(edit(&global, |_| Ok(())).is_err());
        let p = dir.path().join(PROJECT_FILES[0]);
        let doc = edit(&p, |m| {
            m.insert("refero".into(), json!({"enabled": false}));
            Ok(())
        })
        .map_err(|error| format!("{error:?}"))?;
        assert_eq!(doc, json!({"mcpServers": {"refero": {"enabled": false}}}));
        Ok(())
    }
}
