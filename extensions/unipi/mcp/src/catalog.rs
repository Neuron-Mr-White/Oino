//! Server catalog (unipi `src/config/sync.ts`): seed (49 servers), sync from
//! `punkpeye/awesome-mcp-servers`, install enrichment.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// GitHub raw README of `awesome-mcp-servers`.
pub(crate) const GITHUB_RAW_URL: &str =
    "https://raw.githubusercontent.com/punkpeye/awesome-mcp-servers/main/README.md";
/// Bundled seed (`data/seed-servers.json`).
pub(crate) const SEED: &str = include_str!("../assets/seed-servers.json");

/// Pre-filled install configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Install {
    /// Command.
    pub command: String,
    /// Args.
    #[serde(default)]
    pub args: Vec<String>,
    /// Required env vars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<Vec<String>>,
}

/// unipi `CatalogEntry`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// `owner/repo`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Repository URL.
    pub github: String,
    /// Categories.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Language.
    #[serde(default)]
    pub language: String,
    /// `cloud` | `local`.
    #[serde(default)]
    pub scope: String,
    /// Official.
    #[serde(default)]
    pub official: bool,
    /// Install config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Install>,
}

/// unipi `CatalogData`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Catalog {
    /// ISO time.
    pub last_updated: String,
    /// Source id.
    pub source: String,
    /// Count.
    pub total_servers: usize,
    /// Entries.
    pub servers: Vec<Entry>,
}

/// unipi `guessLanguage`.
pub(crate) fn guess_language(name: &str, desc: &str) -> &'static str {
    let text = format!("{name} {desc}").to_lowercase();
    let has = |w: &str| {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| t == w)
    };
    if has("python") || has("uvx") || has("pip") {
        "python"
    } else if has("go") || has("golang") {
        "go"
    } else if has("rust") || has("cargo") {
        "rust"
    } else if has("docker") || has("container") {
        "docker"
    } else {
        "typescript"
    }
}

fn is_local(text: &str) -> bool {
    let t = text.to_lowercase();
    t.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| matches!(w, "local" | "filesystem" | "sqlite" | "postgres" | "docker"))
}

/// Removes `( … )` groups with surrounding whitespace (unipi `/\s*\(.*?\)\s*/g`).
fn strip_parens(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let Some(close_rel) = rest[open..].find(')') else {
            break;
        };
        out.push_str(rest[..open].trim_end());
        rest = rest[open + close_rel + 1..].trim_start();
    }
    out.push_str(rest);
    out
}

/// `[text](url)` → `text`.
fn strip_links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        match (after.find("]("), after.find(']')) {
            (Some(mid), Some(first)) if mid == first => {
                let tail = &after[mid + 2..];
                if let Some(close) = tail.find(')') {
                    out.push_str(&rest[..open]);
                    out.push_str(&after[..mid]);
                    rest = &tail[close + 1..];
                    continue;
                }
                out.push_str(&rest[..=open]);
                rest = after;
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// unipi `parseMarkdownServers`.
pub(crate) fn parse_markdown(markdown: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut category = "uncategorized".to_owned();
    let mut seen: Vec<String> = Vec::new();
    for line in markdown.lines() {
        if let Some(c) = line
            .strip_prefix("##")
            .filter(|r| r.starts_with(char::is_whitespace))
        {
            category = c.trim().to_lowercase();
            continue;
        }
        let Some(rest) = line.strip_prefix("- ").map(str::trim_start) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('[') else {
            continue;
        };
        let Some(close) = rest.find(']') else {
            continue;
        };
        let name = &rest[..close];
        let Some(after) = rest[close + 1..].strip_prefix('(') else {
            continue;
        };
        let Some(end) = after.find(')') else { continue };
        let url = &after[..end];
        if name.is_empty()
            || url.is_empty()
            || !url.contains("github.com")
            || seen.iter().any(|s| s == url)
        {
            continue;
        }
        seen.push(url.to_owned());
        let mut tail = after[end + 1..].trim_start();
        let official = tail.starts_with('⭐');
        if official {
            tail = tail.trim_start_matches('⭐').trim_start();
        }
        let tail = tail.trim_start_matches(['-', '–', '—']).trim_start();
        let description: String = strip_links(&strip_parens(tail))
            .trim()
            .chars()
            .take(200)
            .collect();
        let id = url
            .split("github.com/")
            .nth(1)
            .map(|p| p.split('/').take(2).collect::<Vec<_>>())
            .filter(|parts| parts.len() == 2 && parts.iter().all(|p| !p.is_empty()))
            .map_or_else(|| url.to_owned(), |p| p.join("/"));
        let local = is_local(&format!("{name} {description}"));
        out.push(Entry {
            id,
            name: name.to_owned(),
            description: if description.is_empty() {
                format!("MCP server: {name}")
            } else {
                description.clone()
            },
            github: url.to_owned(),
            categories: vec![category.clone()],
            language: guess_language(name, &description).to_owned(),
            scope: if local { "local" } else { "cloud" }.to_owned(),
            official,
            install: None,
        });
    }
    out
}

/// unipi `KNOWN_INSTALLS`.
#[allow(clippy::too_many_lines, reason = "one data table")]
pub(crate) fn known_installs() -> Vec<(&'static str, Install)> {
    let i = |cmd: &str, args: &[&str], env: &[&str]| Install {
        command: cmd.to_owned(),
        args: args.iter().map(ToString::to_string).collect(),
        env_vars: (!env.is_empty()).then(|| env.iter().map(ToString::to_string).collect()),
    };
    let npx = |pkg: &str, env: &[&str]| i("npx", &["-y", pkg], env);
    vec![
        (
            "github",
            i(
                "docker",
                &[
                    "run",
                    "-i",
                    "--rm",
                    "-e",
                    "GITHUB_PERSONAL_ACCESS_TOKEN",
                    "ghcr.io/github/github-mcp-server",
                ],
                &["GITHUB_PERSONAL_ACCESS_TOKEN"],
            ),
        ),
        ("playwright", npx("@playwright/mcp", &[])),
        (
            "brave-search",
            npx(
                "@modelcontextprotocol/server-brave-search",
                &["BRAVE_API_KEY"],
            ),
        ),
        (
            "filesystem",
            npx("@modelcontextprotocol/server-filesystem", &[]),
        ),
        (
            "postgres",
            npx(
                "@modelcontextprotocol/server-postgres",
                &["POSTGRES_CONNECTION_STRING"],
            ),
        ),
        ("sqlite", npx("@modelcontextprotocol/server-sqlite", &[])),
        ("memory", npx("@modelcontextprotocol/server-memory", &[])),
        ("fetch", npx("@modelcontextprotocol/server-fetch", &[])),
        (
            "puppeteer",
            npx("@modelcontextprotocol/server-puppeteer", &[]),
        ),
        (
            "supabase",
            npx("@supabase/mcp-server-supabase", &["SUPABASE_ACCESS_TOKEN"]),
        ),
        ("docker", npx("@modelcontextprotocol/server-docker", &[])),
        (
            "gitlab",
            npx(
                "@modelcontextprotocol/server-gitlab",
                &["GITLAB_PERSONAL_ACCESS_TOKEN"],
            ),
        ),
        ("linear", npx("mcp-linear", &["LINEAR_API_KEY"])),
        (
            "notion",
            npx("@notionhq/notion-mcp-server", &["NOTION_API_KEY"]),
        ),
        (
            "slack",
            npx(
                "@modelcontextprotocol/server-slack",
                &["SLACK_BOT_TOKEN", "SLACK_TEAM_ID"],
            ),
        ),
        ("sentry", npx("@sentry/mcp-server", &["SENTRY_AUTH_TOKEN"])),
        (
            "google-maps",
            npx(
                "@modelcontextprotocol/server-google-maps",
                &["GOOGLE_MAPS_API_KEY"],
            ),
        ),
        (
            "google-drive",
            npx(
                "@modelcontextprotocol/server-gdrive",
                &["GOOGLE_DRIVE_CREDENTIALS"],
            ),
        ),
        (
            "aws-kb-retrieval",
            npx(
                "@modelcontextprotocol/server-aws-kb-retrieval",
                &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_REGION"],
            ),
        ),
        (
            "everything",
            npx("@modelcontextprotocol/server-everything", &[]),
        ),
        (
            "sequential-thinking",
            npx("@modelcontextprotocol/server-sequential-thinking", &[]),
        ),
        ("computer-use", npx("@anthropic/mcp-computer-use", &[])),
        ("figma", npx("mcp-figma", &["FIGMA_ACCESS_TOKEN"])),
        ("discord", npx("mcp-discord", &["DISCORD_BOT_TOKEN"])),
        ("tavily", npx("mcp-tavily", &["TAVILY_API_KEY"])),
        (
            "firecrawl",
            npx("mcp-server-firecrawl", &["FIRECRAWL_API_KEY"]),
        ),
        (
            "cloudflare",
            npx(
                "@cloudflare/mcp-server-cloudflare",
                &["CLOUDFLARE_API_TOKEN"],
            ),
        ),
        ("vercel", npx("mcp-vercel", &["VERCEL_API_TOKEN"])),
        ("neon", npx("mcp-server-neon", &["NEON_API_KEY"])),
        ("railway", npx("mcp-server-railway", &["RAILWAY_API_TOKEN"])),
        ("time-mcp", npx("time-mcp", &[])),
        ("weather-mcp", npx("weather-mcp", &[])),
        ("exa", npx("mcp-server-exa", &["EXA_API_KEY"])),
    ]
}

/// unipi `enrichWithInstallInfo`.
pub(crate) fn enrich(entries: Vec<Entry>) -> Vec<Entry> {
    let known = known_installs();
    entries
        .into_iter()
        .map(|mut e| {
            if e.install.is_none() {
                let lower_name = e
                    .name
                    .to_lowercase()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join("-");
                let lower_id = e.id.to_lowercase();
                e.install = known
                    .iter()
                    .find(|(p, _)| {
                        lower_name.contains(p)
                            || lower_id.contains(p)
                            || lower_name.replace('-', "").contains(&p.replace('-', ""))
                    })
                    .map(|(_, i)| i.clone());
            }
            e
        })
        .collect()
}

/// unipi `syncCatalog` minus I/O.
pub(crate) fn from_markdown(markdown: &str, now_iso: &str) -> Catalog {
    let servers = enrich(parse_markdown(markdown));
    Catalog {
        last_updated: now_iso.to_owned(),
        source: "github:punkpeye/awesome-mcp-servers".into(),
        total_servers: servers.len(),
        servers,
    }
}

/// unipi `loadCatalog`: cached (non-empty) → seed → empty.
pub(crate) fn load(cached: Option<&str>, now_iso: &str) -> Catalog {
    cached
        .and_then(|t| serde_json::from_str::<Catalog>(t).ok())
        .filter(|c| !c.servers.is_empty())
        .or_else(|| serde_json::from_str::<Catalog>(SEED).ok())
        .unwrap_or_else(|| Catalog {
            last_updated: now_iso.to_owned(),
            source: "seed".into(),
            total_servers: 0,
            servers: Vec::new(),
        })
}

/// Filters by a case-insensitive query over name, description, categories and id.
pub(crate) fn search<'a>(catalog: &'a Catalog, query: &str) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    catalog
        .servers
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.id.to_lowercase().contains(&q)
                || e.categories.iter().any(|c| c.to_lowercase().contains(&q))
        })
        .collect()
}

/// Config template for the add editor (`{"<name>": {command, args, env}}`).
pub(crate) fn template(entry: Option<&Entry>) -> (String, Value) {
    let Some(e) = entry else {
        return (
            "my-server".into(),
            json!({"command": "npx", "args": ["-y", "package-name"]}),
        );
    };
    let name: String = e
        .name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let name = name
        .trim_end_matches("-mcp-server")
        .trim_end_matches("-mcp")
        .to_owned();
    let def = match &e.install {
        Some(i) => {
            let mut v = json!({"command": i.command, "args": i.args});
            if let Some(vars) = &i.env_vars {
                let env: serde_json::Map<String, Value> = vars
                    .iter()
                    .map(|k| (k.clone(), json!(format!("${{{k}}}"))))
                    .collect();
                v["env"] = Value::Object(env);
            }
            v
        }
        None => {
            json!({"command": "npx", "args": ["-y", "package-name"], "description": e.description})
        }
    };
    (
        if name.is_empty() {
            "server".into()
        } else {
            name
        },
        def,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# Awesome\n\n## 🗂️ File Systems\n- [Filesystem](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem) ⭐ - Local filesystem access (read/write) via [MCP](https://x)\n- [Dup](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem) - again\n- [NotGitHub](https://gitlab.com/x/y) - nope\n\n## Databases\n- [pg-tool](https://github.com/acme/pg-tool) – A python postgres helper\n* [Star](https://github.com/a/b) - star bullets are not parsed\n";

    #[test]
    fn parses_awesome_list() {
        let e = parse_markdown(MD);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].id, "modelcontextprotocol/servers");
        assert_eq!(e[0].name, "Filesystem");
        assert!(e[0].official);
        // Same as unipi's regex chain: parens (and link targets) go first, eating spaces.
        assert_eq!(e[0].description, "Local filesystem accessvia [MCP]");
        assert_eq!(e[0].categories, ["🗂️ file systems"]);
        assert_eq!(e[0].scope, "local");
        assert_eq!(e[1].language, "python");
        assert_eq!(e[1].categories, ["databases"]);
        assert!(!e[1].official);
    }

    #[test]
    fn languages_and_enrichment() {
        assert_eq!(guess_language("x", "written in Go"), "go");
        assert_eq!(guess_language("cargo-mcp", ""), "rust");
        assert_eq!(guess_language("x", "runs in a docker container"), "docker");
        assert_eq!(
            guess_language("goal", "typescript"),
            "typescript",
            "word match, not substring"
        );
        let c = from_markdown(MD, "T");
        assert_eq!(c.total_servers, 2);
        assert_eq!(c.source, "github:punkpeye/awesome-mcp-servers");
        assert_eq!(
            c.servers[0].install.as_ref().map(|i| i.args.clone()),
            Some(vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into()
            ])
        );
        assert_eq!(c.servers[1].install, None, "no known install pattern");
    }

    #[test]
    fn seed_and_cache() {
        let seed = load(None, "T");
        assert_eq!(seed.source, "seed");
        assert_eq!(seed.servers.len(), 49);
        assert_eq!(seed.servers[0].id, "github/github-mcp-server");
        let cached = serde_json::to_string(&from_markdown(MD, "T")).unwrap_or_default();
        assert_eq!(load(Some(&cached), "T").servers.len(), 2);
        assert_eq!(
            load(Some("{\"servers\":[]}"), "T").source,
            "seed",
            "empty cache falls back"
        );
        assert_eq!(
            search(&seed, "github").first().map(|e| e.id.as_str()),
            Some("github/github-mcp-server")
        );
        assert_eq!(search(&seed, "").len(), 49);
    }

    #[test]
    fn templates() {
        let seed = load(None, "T");
        let (name, def) = template(seed.servers.first());
        assert_eq!(name, "github");
        assert_eq!(def["command"], "docker");
        assert_eq!(
            def["env"]["GITHUB_PERSONAL_ACCESS_TOKEN"],
            "${GITHUB_PERSONAL_ACCESS_TOKEN}"
        );
        let (name, def) = template(None);
        assert_eq!(name, "my-server");
        assert_eq!(def["command"], "npx");
    }
}
