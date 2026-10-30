//! Hibernate-probe fixture: records `session_shutdown` reasons (core spec §2.2
//! E-HIBERNATE, WP-C8).
//!
//! Subscribes to `session_shutdown` and appends a session `custom` entry
//! `hibernate-probe.shutdown` with the reason — so a test can prove a hibernating
//! subagent told its extensions (and that guests can still write during the event).

use oino_extension::prelude::*;

/// Fixture state.
#[derive(Default)]
struct HibernateProbe;

impl Extension for HibernateProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.on(EventKind::SessionShutdown, |_ext, ev: Event, _cx| async move {
            let reason = ev
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            session::append_entry(
                "hibernate-probe.shutdown",
                Some(&json!({"reason": reason})),
            )?;
            Ok(None::<Value>)
        });
        Ok(())
    }
}

oino_extension::export_extension!(HibernateProbe);
