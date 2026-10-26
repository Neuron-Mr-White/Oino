//! Background-task one-liner (unipi `src/process-line.ts`) from the sticky `unipi:bg:tasks`
//! topic (replaces the shared task registry, unipi-port §2.2).
//!
//! `● 2 running  ● 1 stopped  ● 1 failed  ● 3 done` — green/yellow/red/grey dots, zero buckets
//! omitted, nothing at all when no tasks exist.

use serde_json::{Value, json};

/// Widget key.
pub const WIDGET_KEY: &str = "footer-top";

/// Counts by display bucket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Running.
    pub running: u32,
    /// Killed / stopped.
    pub stopped: u32,
    /// Failed.
    pub failed: u32,
    /// Completed.
    pub done: u32,
}

/// Counts tasks from the topic payload (`[task]` or `{tasks: [task]}` with `status`/`state`);
/// `None` when no registry has been published.
#[must_use]
pub fn count(payload: Option<&Value>) -> Option<Counts> {
    let p = payload?;
    let tasks = p
        .as_array()
        .or_else(|| p.get("tasks").and_then(Value::as_array))?;
    let mut c = Counts::default();
    for t in tasks {
        let status = t
            .get("status")
            .or_else(|| t.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        match status {
            "running" => c.running += 1,
            "killed" | "stopped" => c.stopped += 1,
            "failed" => c.failed += 1,
            "completed" | "done" => c.done += 1,
            _ => {}
        }
    }
    Some(c)
}

/// The widget request (`None` lines clear it).
#[must_use]
pub fn widget_request(counts: Option<Counts>, color: bool) -> Value {
    let Some(c) = counts else {
        return clear_request();
    };
    let buckets = [
        (c.running, "running", "success"),
        (c.stopped, "stopped", "warning"),
        (c.failed, "failed", "error"),
        (c.done, "done", "muted"),
    ];
    let mut spans = Vec::new();
    let mut plain = Vec::new();
    for (n, label, tone) in buckets.into_iter().filter(|(n, _, _)| *n > 0) {
        if !spans.is_empty() {
            spans.push(json!({"text": "  "}));
        }
        spans.push(if color {
            json!({"text": "●", "tone": tone})
        } else {
            json!({"text": "●"})
        });
        spans.push(json!({"text": format!(" {n} {label}")}));
        plain.push(format!("● {n} {label}"));
    }
    if plain.is_empty() {
        return clear_request();
    }
    json!({
        "method": "setWidget",
        "widgetKey": WIDGET_KEY,
        "widgetPlacement": "aboveEditor",
        "widgetLines": [plain.join("  ")],
        "widgetSpec": {"t": "text", "value": spans, "wrap": false, "align": "center"},
    })
}

/// Clears the widget.
#[must_use]
pub fn clear_request() -> Value {
    json!({"method": "setWidget", "widgetKey": WIDGET_KEY})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(count(None), None);
        let p = json!([{"status": "running"}, {"status": "killed"}, {"status": "failed"}, {"status": "completed"}, {"state": "done"}, {"status": "queued"}]);
        assert_eq!(
            count(Some(&p)),
            Some(Counts {
                running: 1,
                stopped: 1,
                failed: 1,
                done: 2
            })
        );
        assert_eq!(count(Some(&json!({"tasks": []}))), Some(Counts::default()));
    }

    #[test]
    fn widget_lines() {
        assert_eq!(widget_request(None, true), clear_request());
        assert_eq!(
            widget_request(Some(Counts::default()), true),
            clear_request()
        );
        let w = widget_request(
            Some(Counts {
                running: 2,
                stopped: 0,
                failed: 1,
                done: 0,
            }),
            true,
        );
        assert_eq!(w["widgetLines"][0], "● 2 running  ● 1 failed");
        assert_eq!(w["widgetSpec"]["value"][0]["tone"], "success");
        assert_eq!(w["widgetSpec"]["value"][3]["tone"], "error");
        assert_eq!(w["widgetPlacement"], "aboveEditor");
    }
}
