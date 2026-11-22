//! Watcher-backed topic invalidations. Payload reads use the existing authorized ops.
use crate::topic_hub::{SourceGuard, TopicHub};
use notify::Watcher;
use serde_json::json;
use std::{path::Path, time::Duration};

/// Topics with complete change sources. Others deliberately keep their legacy polling.
#[must_use]
pub fn supported(topic: &str) -> bool {
    matches!(
        unipi_proto::Topic::parse(topic),
        Some(
            unipi_proto::Topic::Plain("sessions")
                | unipi_proto::Topic::Keyed("board" | "git" | "fs", _)
        )
    )
}

#[must_use]
pub fn start(topic: &str) -> Option<SourceGuard> {
    let parsed = unipi_proto::Topic::parse(topic)?;
    match parsed {
        unipi_proto::Topic::Keyed("board", _) => {
            let watchers = crate::board_cli::BoardWatchers::shared();
            if !watchers.set(true) {
                return None;
            }
            Some(SourceGuard::new(move || {
                watchers.set(false);
            }))
        }
        unipi_proto::Topic::Keyed("fs", dir) => {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    // Reads must not cause a feedback loop of invalidation/refetch.
                    if event
                        .as_ref()
                        .is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_)))
                    {
                        let _ = tx.try_send(());
                    }
                })
                .ok()?;
            watcher
                .watch(Path::new(&dir), notify::RecursiveMode::NonRecursive)
                .ok()?;
            let topic = topic.to_owned();
            let task = tokio::spawn(async move {
                let _watcher = watcher;
                while rx.recv().await.is_some() {
                    // Bounded debounce: coalesce a burst without starving sustained writes.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while rx.try_recv().is_ok() {}
                    TopicHub::shared().publish(&topic, json!({"invalidate": true}));
                }
            });
            Some(SourceGuard::task(task))
        }
        unipi_proto::Topic::Keyed("git", dir) => {
            let topic = topic.to_owned();
            let watchers = crate::git_watch::GitWatchers::shared();
            let mut events = watchers.subscribe();
            // start() runs on the blocking pool; report setup failure before advertising push.
            let root =
                tokio::runtime::Handle::current().block_on(watchers.set(dir.into(), true))?;
            let task = tokio::spawn(async move {
                // Drop ref even when the source task is aborted.
                struct GitRef(String);
                impl Drop for GitRef {
                    fn drop(&mut self) {
                        let root = self.0.clone();
                        tokio::spawn(async move {
                            crate::git_watch::GitWatchers::shared()
                                .set(root.into(), false)
                                .await;
                        });
                    }
                }
                let _guard = GitRef(root.clone());
                loop {
                    match events.recv().await {
                        Ok(event) if event.root == root => {
                            TopicHub::shared().publish(&topic, json!({"invalidate": true}));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            TopicHub::shared().publish(&topic, json!({"invalidate": true}));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        _ => {}
                    }
                }
            });
            Some(SourceGuard::task(task))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn missing_fs_and_git_sources_report_failure() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        for family in ["fs", "git"] {
            let topic = format!("{family}:{}", missing.display());
            assert!(
                tokio::task::spawn_blocking(move || super::start(&topic).is_none())
                    .await
                    .unwrap()
            );
        }
    }
}
