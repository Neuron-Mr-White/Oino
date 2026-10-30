//! `/browser` (browser-control.md §11.2).

use std::fmt::Write as _;

use oino_extension::browser;
use oino_extension::prelude::*;

use crate::BrowserExt;

/// Text of `/browser` (status line + this session's tabs).
#[must_use]
pub fn status_text(st: &Value) -> String {
    if st["available"] != json!(true) {
        let reason = st["reason"].as_str().unwrap_or("unknown");
        let hint = match reason {
            "disabled" => "Browser control is off (Settings → Browser).",
            "no_engine" => {
                "No Chrome-family browser found: install Google Chrome or Chromium, or set browser.executablePath."
            }
            "cloak_not_installed" => {
                "CloakBrowser is not installed (Settings → Browser → Install CloakBrowser), or set browser.engine to auto."
            }
            _ => "The browser is unavailable.",
        };
        return format!("Browser unavailable ({reason}). {hint}");
    }
    let mut out = format!(
        "{} {}· {} · profile {}{}",
        st["product"].as_str().unwrap_or("Chrome"),
        st["version"]
            .as_str()
            .map(|v| format!("({v}) "))
            .unwrap_or_default(),
        st["display"].as_str().unwrap_or("headless"),
        st["sessionProfile"].as_str().unwrap_or("default"),
        if st["stealth"] == json!(true) {
            " · stealth"
        } else {
            ""
        }
    );
    let tabs = st["tabs"].as_array().cloned().unwrap_or_default();
    if tabs.is_empty() {
        out.push_str("\nNo tabs in this conversation.");
    }
    for t in tabs {
        let _ = write!(
            out,
            "\n{} · {} · {}{} · {}",
            t["id"].as_str().unwrap_or("?"),
            t["owner"]["kind"].as_str().unwrap_or("?"),
            t["lifecycle"].as_str().unwrap_or("?"),
            if t["controlledBy"] == json!("user") {
                " (you have control)"
            } else {
                ""
            },
            t["url"].as_str().unwrap_or_default()
        );
    }
    out
}

async fn run(args: String) -> Result<()> {
    let mut words = args.split_whitespace();
    let sub = words.next().unwrap_or("");
    let arg = words.next();
    let reply = match sub {
        "" | "status" => status_text(&browser::status()),
        "profile" => match arg {
            Some(name) => op(json!({"op": "profile", "action": "use", "name": name})).await,
            None => op(json!({"op": "profile", "action": "status"})).await,
        },
        "release" => op(json!({"op": "release", "tab": arg})).await,
        "close-tabs" => op(json!({"op": "close_agent_tabs"})).await,
        "show" | "hide" | "install-cloak" => format!(
            "/browser {sub}: use Settings → Browser in Oino Desktop (or `oino browser {sub}`)."
        ),
        other => format!(
            "Unknown /browser subcommand {other:?}. Try: status, profile [name], release [tab], close-tabs."
        ),
    };
    ui::notify(&reply, "info")?;
    Ok(())
}

async fn op(cmd: Value) -> String {
    match browser::execute(&cmd).await {
        Ok((r, _)) => r["text"].as_str().unwrap_or_default().to_owned(),
        Err(e) => format!("Browser unavailable: {e}"),
    }
}

/// The `/browser` command.
#[must_use]
pub fn command() -> Command<BrowserExt> {
    Command::new("browser")
        .description("In-app browser: status, profile [name], release [tab], close-tabs")
        .argument_hint("[status|profile [name]|release [tab]|close-tabs]")
        .run(|_ext, args, _cx| run(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        assert!(
            status_text(&json!({"available": false, "reason": "no_engine"}))
                .contains("install Google Chrome")
        );
        let s = status_text(
            &json!({"available": true, "product": "Chromium", "version": "Chrome/146", "display": "headless",
            "sessionProfile": "default", "stealth": false,
            "tabs": [{"id": "t1", "owner": {"kind": "agent", "session": "s"}, "lifecycle": "active", "controlledBy": "user", "url": "https://x.dev/"}]}),
        );
        assert_eq!(
            s,
            "Chromium (Chrome/146) · headless · profile default\nt1 · agent · active (you have control) · https://x.dev/"
        );
    }
}
