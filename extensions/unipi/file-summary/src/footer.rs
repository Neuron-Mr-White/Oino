//! The turn-footer node tree (ui-protocol §6.7): "3 files +12 −04" plus one row per touched
//! file. The app renders this reserved surface natively (`unipi.file-summary/turn-files`,
//! WP-A27); TUI and other non-native frontends render this tree.

use oino_extension::context::Ctx;
use oino_extension::session;
use oino_extension::ui::widgets::prelude::*;
use serde_json::Value;

use crate::services::{run_key, summary};
use crate::snapshot::finish as snapshot_finish;
use crate::store::FileRecord;

/// Renderer key, matching the manifest's reserved id and the `turn-footer` registration.
pub const KEY: &str = "files";

fn status(file: &FileRecord, undone: bool) -> &'static str {
    if file.is_skipped() {
        return "skipped";
    }
    if undone {
        return "undone";
    }
    if !file.existed {
        return "new";
    }
    if file.after_sha.is_none() {
        return "deleted";
    }
    "modified"
}

/// The footer node for one turn. `None` when the turn touched no file (the footer disappears).
#[must_use]
pub fn node(files: &[FileRecord], undone_paths: &[String]) -> Option<Node> {
    if files.is_empty() {
        return None;
    }
    let adds: u64 = files.iter().map(|file| file.add).sum();
    let dels: u64 = files.iter().map(|file| file.del).sum();
    let count = files.len();
    let plural = if count == 1 { "" } else { "s" };
    let mut header = vec![Span::from(format!("{count} file{plural} ")).fg(Color::Muted)];
    header.push(Span::from(format!("+{adds:02}")).fg(Color::Success));
    header.push(Span::from(" "));
    header.push(Span::from(format!("\u{2212}{dels:02}")).fg(Color::Error));
    if files
        .iter()
        .any(|file| undone_paths.iter().any(|path| path == &file.path))
    {
        header.push(Span::from(" · undone").fg(Color::Dim));
    }
    let items: Vec<ListItem> = files
        .iter()
        .map(|file| {
            let undone = undone_paths.iter().any(|path| path == &file.path);
            ListItem::new(format!(
                "{}  +{} \u{2212}{}  {}",
                file.path,
                file.add,
                file.del,
                status(file, undone)
            ))
        })
        .collect();
    Some(
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
            .children([
                Paragraph::new(Line::from(header)).into(),
                List::new(items).into(),
            ])
            .into(),
    )
}

/// The guest `render` export for `{target:"turnFooter", key}`. Returns `None` for another key
/// and for a turn without file changes.
///
/// A running turn is re-requested at most every 500 ms (ui-protocol §6.7): its `+X −Y` are
/// computed from the snapshots and the files as they are now, so the live footer is not a row
/// of zeros. Finished turns use the counts recorded at `agent_end`.
#[must_use]
pub fn render(request: &Value, cx: &Ctx) -> Option<Value> {
    if request.get("key").and_then(Value::as_str) != Some(KEY) {
        return None;
    }
    let turn_entry_id = request
        .get("turnEntryId")
        .and_then(Value::as_str)
        .filter(|turn| !turn.is_empty());
    let mut summary = match turn_entry_id {
        Some(turn_entry_id) => summary(turn_entry_id),
        None => run_key()?,
    };
    if summary.running {
        let session_id = session::session_id();
        let cwd = cx.cwd();
        summary.record.files = summary
            .record
            .files
            .into_iter()
            .filter_map(|file| snapshot_finish(&session_id, &cwd, file))
            .collect();
    }
    let node = node(&summary.record.files, &summary.undone_paths)?;
    serde_json::to_value(node).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use super::*;
    use crate::store::{TOO_LARGE, UNREADABLE};

    fn file(path: &str, existed: bool, add: u64, del: u64) -> FileRecord {
        FileRecord {
            path: path.into(),
            existed,
            before_sha: existed.then(|| "aa".to_owned()),
            after_sha: Some("bb".into()),
            add,
            del,
            skipped: None,
        }
    }

    fn lines(files: &[FileRecord], undone: &[String], width: u16) -> Vec<String> {
        let node = node(files, undone).unwrap();
        oino_extension::ui::widgets::plain::to_lines(&node, width)
    }

    #[test]
    fn empty_turn_has_no_footer() {
        assert!(node(&[], &[]).is_none());
    }

    #[test]
    fn header_counts_and_pads() {
        let files = [
            file("src/a.rs", true, 10, 2),
            file("src/b.rs", false, 2, 0),
            file("src/c.rs", true, 0, 2),
        ];
        let out = lines(&files, &[], 100);
        assert_eq!(out[0], "3 files +12 \u{2212}04");
        assert_eq!(out[1], "• src/a.rs  +10 \u{2212}2  modified");
        assert_eq!(out[2], "• src/b.rs  +2 \u{2212}0  new");
        assert_eq!(out[3], "• src/c.rs  +0 \u{2212}2  modified");
    }

    #[test]
    fn golden_at_40_and_100_columns() {
        let files = [
            file("src/components/timeline/TurnFilesFooter.tsx", true, 12, 4),
            FileRecord {
                path: "assets/big.bin".into(),
                existed: true,
                add: 0,
                del: 0,
                skipped: Some(TOO_LARGE.into()),
                ..FileRecord::default()
            },
        ];
        assert_eq!(
            lines(&files, &[], 100),
            [
                "2 files +12 −04",
                "• src/components/timeline/TurnFilesFooter.tsx  +12 −4  modified",
                "• assets/big.bin  +0 −0  skipped",
            ]
        );
        assert_eq!(
            lines(&files, &[], 40),
            [
                "2 files +12 −04",
                "•",
                "src/components/timeline/TurnFilesFooter.",
                "tsx  +12 −4  modified",
                "• assets/big.bin  +0 −0  skipped",
            ]
        );
    }

    #[test]
    fn undone_and_unreadable_states() {
        let mut unreadable = file("x", true, 0, 0);
        unreadable.skipped = Some(UNREADABLE.into());
        let files = [file("a", true, 1, 0), unreadable];
        let out = lines(&files, &["a".to_owned()], 100);
        assert_eq!(out[0], "2 files +01 −00 · undone");
        assert_eq!(out[1], "• a  +1 −0  undone");
        assert_eq!(out[2], "• x  +0 −0  skipped");
    }
}
