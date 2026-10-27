//! Navigation policy (browser-control.md §10.1), the sensitive-action heuristic (§10.2) and the
//! take-over gate (§7.4). Pure functions.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};

use oino_browser_protocol::{BrowserCommand, ControlledBy};

/// Why a URL is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Blocked(pub(crate) String);

fn globs(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Origin (`scheme://host[:port]`) of `url`.
pub(crate) fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    ))
}

/// Hostname of `url` (no port).
pub(crate) fn hostname(url: &str) -> Option<String> {
    let o = origin(url)?;
    let host = o.split_once("://")?.1;
    let host = if host.starts_with('[') {
        host.split_once(']')
            .map_or(host, |(h, _)| h)
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    Some(host.to_owned())
}

/// Loopback hosts (heuristics are off there).
pub(crate) fn is_loopback(url: &str) -> bool {
    hostname(url)
        .is_some_and(|h| h == "localhost" || h.starts_with("127.") || h == "::1" || h == "0.0.0.0")
}

/// Agent navigation check.
pub(crate) fn check_agent_url(
    url: &str,
    cwd: &Path,
    allowed: &[String],
    blocked: &[String],
) -> Result<(), Blocked> {
    let lower = url.to_ascii_lowercase();
    if lower == "about:blank" {
        return Ok(());
    }
    let scheme = lower.split(':').next().unwrap_or_default();
    match scheme {
        "http" | "https" => {}
        "data" => {
            if lower.starts_with("data:text/html") {
                return Err(Blocked(
                    "top-level data:text/html navigation is not allowed".into(),
                ));
            }
            return Ok(());
        }
        "file" => {
            let path = url.trim_start_matches("file://");
            let ok = Path::new(path)
                .canonicalize()
                .ok()
                .zip(cwd.canonicalize().ok())
                .is_some_and(|(p, c)| p.starts_with(c));
            return if ok {
                Ok(())
            } else {
                Err(Blocked(
                    "file: URLs are allowed only below the session's working directory".into(),
                ))
            };
        }
        _ => {
            return Err(Blocked(format!(
                "the {scheme}: scheme is not allowed for agent navigation (use http, https, about:blank)"
            )));
        }
    }
    let o = origin(url).unwrap_or_default();
    let host = hostname(url).unwrap_or_default();
    if globs(blocked).is_match(&o) || globs(blocked).is_match(&host) {
        return Err(Blocked(format!("{o} is blocked by browser.blockedOrigins")));
    }
    if !allowed.is_empty() && !globs(allowed).is_match(&o) && !globs(allowed).is_match(&host) {
        return Err(Blocked(format!("{o} is not in browser.allowedOrigins")));
    }
    Ok(())
}

/// User navigation check (only `blockedOrigins`).
pub(crate) fn check_user_url(url: &str, blocked: &[String]) -> Result<(), Blocked> {
    let o = origin(url).unwrap_or_default();
    let host = hostname(url).unwrap_or_default();
    if !o.is_empty() && (globs(blocked).is_match(&o) || globs(blocked).is_match(&host)) {
        return Err(Blocked(format!("{o} is blocked by browser.blockedOrigins")));
    }
    Ok(())
}

const SENSITIVE_WORDS: &[&str] = &[
    "buy",
    "pay",
    "purchase",
    "place order",
    "checkout",
    "check out",
    "subscribe",
    "delete",
    "remove account",
    "transfer",
    "send",
    "publish",
    "post",
    "confirm payment",
];

/// Host heuristic forcing `sensitive` (§10.2): target accessible name matches the word list.
pub(crate) fn looks_sensitive(name: &str) -> bool {
    let lower = name.to_lowercase();
    SENSITIVE_WORDS.iter().any(|w| {
        lower.match_indices(w).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + w.len()..].chars().next();
            before.is_none_or(|c| !c.is_alphanumeric())
                && after.is_none_or(|c| !c.is_alphanumeric())
        })
    })
}

/// Take-over gate: driving commands are refused while the user controls the tab.
pub(crate) fn gate(controlled_by: ControlledBy, cmd: &BrowserCommand) -> bool {
    controlled_by == ControlledBy::Agent || !cmd.is_driving()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_urls() {
        let cwd = std::env::temp_dir();
        assert!(check_agent_url("https://example.com/a", &cwd, &[], &[]).is_ok());
        assert!(check_agent_url("about:blank", &cwd, &[], &[]).is_ok());
        assert!(check_agent_url("chrome://settings", &cwd, &[], &[]).is_err());
        assert!(check_agent_url("javascript:alert(1)", &cwd, &[], &[]).is_err());
        assert!(check_agent_url("data:text/html,<b>x</b>", &cwd, &[], &[]).is_err());
        assert!(check_agent_url("file:///etc/passwd", &cwd, &[], &[]).is_err());
        let blocked = vec!["*.evil.com".to_owned()];
        assert!(check_agent_url("https://a.evil.com/", &cwd, &[], &blocked).is_err());
        let allowed = vec!["https://docs.rs".to_owned()];
        assert!(check_agent_url("https://docs.rs/x", &cwd, &allowed, &[]).is_ok());
        assert!(check_agent_url("https://crates.io/", &cwd, &allowed, &[]).is_err());
        assert!(check_user_url("https://a.evil.com/", &blocked).is_err());
    }

    #[test]
    fn helpers() {
        assert_eq!(
            origin("https://u:p@Ex.com:8080/x?y").as_deref(),
            Some("https://ex.com:8080")
        );
        assert_eq!(
            hostname("http://localhost:3000/").as_deref(),
            Some("localhost")
        );
        assert!(is_loopback("http://127.0.0.1:9/"));
        assert!(looks_sensitive("Place order"));
        assert!(looks_sensitive("Send"));
        assert!(!looks_sensitive("Sender settings"));
        assert!(!looks_sensitive("Posts archive"));
        let click = BrowserCommand::from_tool("browser_click", &serde_json::json!({"ref": "e1"}))
            .unwrap_or(BrowserCommand::Status);
        assert!(!gate(ControlledBy::User, &click));
        assert!(gate(ControlledBy::User, &BrowserCommand::Status));
        assert!(gate(ControlledBy::Agent, &click));
    }
}
