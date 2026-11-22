//! `owner_messages` — the lead's read-only window into the owner's verbatim
//! words (OIN-83 lossless flow). A chat that sent work to the lead's project
//! (a ticket's `superagent` origin) can be paged newest first or searched by
//! keyword (BM25, the `session_recall` scorer); everything else is refused.

use oino_ai::{AgentMessage, UserContent, UserContentField};
use serde_json::{json, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};
use unipi_kanboard::{
    board::Board,
    model::{OriginKind, Status},
    Layout, Project,
};

/// The tool spec schema (shared by the launch and reopen paths).
pub(crate) fn spec() -> (&'static str, &'static str, Value, Vec<&'static str>) {
    (
        "owner_messages",
        "Read the owner's own messages from a Superagent chat, word for word, newest first. Use it when a ticket is unclear or seems to miss detail, before you ask the owner. With `query` you can search the chat by keywords instead of paging (best match first).",
        json!({
            "chatId":{"type":"string","description":"Superagent chat id from a ticket's origin; default: the origin of your newest ticket."},
            "query":{"type":"string","description":"Keywords to find in the owner's messages (BM25 ranking, best match first). Omit to page newest first."},
            "before":{"type":"string","description":"Cursor from the previous page."},
            "limit":{"type":"integer","minimum":1,"maximum":50,"default":20}
        }),
        Vec::new(),
    )
}

/// The last branch entry whose message is a user turn with no `author` —
/// the id a `superagent:<sid>:<entry>` origin names. Host-authored custom
/// messages and injected context carry an author and never qualify.
#[must_use]
pub fn latest_owner_entry(session: &oino_core::session::AgentSession) -> Option<String> {
    session.with_manager(|m| {
        let branch = m.branch(None);
        latest_owner_entry_in(&branch)
    })
}

/// Image bytes from the latest owner turn, resolving mediaRefs through the host blob store.
pub(crate) fn latest_owner_images_from(session_id: &str) -> Vec<(String, Vec<u8>)> {
    use base64::Engine as _;
    let Some(runtime) = crate::superagent_session::runtime(session_id) else {
        return Vec::new();
    };
    let service = crate::sessions::SessionService::shared();
    let blobs = service.as_ref().map(|s| s.hub().blobs_dir());
    runtime.session().with_manager(|m| {
        let branch = m.branch(None);
        let Some(entry) = branch.iter().rev().find(|entry| matches!(entry.message(), Some(AgentMessage::User(user)) if user.author.is_none())) else { return Vec::new() };
        let Some(AgentMessage::User(user)) = entry.message() else { return Vec::new() };
        let UserContentField::Blocks(blocks) = &user.content else { return Vec::new() };
        blocks.iter().filter_map(|block| {
            let UserContent::Image(image) = block else { return None };
            let bytes = if image.data.len() == 64 && image.data.bytes().all(|b| b.is_ascii_hexdigit()) {
                std::fs::read(blobs.as_ref()?.join(&image.data)).ok()?
            } else { base64::engine::general_purpose::STANDARD.decode(&image.data).ok()? };
            Some((image.mime_type.clone(), bytes))
        }).collect()
    })
}

/// The same lookup for a live Superagent chat, resolved by session id through
/// the chat runtime registry (read-only; no manager lease is taken).
#[must_use]
pub fn latest_owner_entry_from(session_id: &str) -> Option<String> {
    crate::superagent_session::runtime(session_id)
        .and_then(|runtime| latest_owner_entry(&runtime.session()))
}

/// Shared scan over branch entries, newest first.
fn latest_owner_entry_in(entries: &[&oino_session::SessionEntry]) -> Option<String> {
    entries
        .iter()
        .rev()
        .find_map(|entry| match entry.message() {
            Some(AgentMessage::User(user)) if user.author.is_none() => Some(entry.id.to_string()),
            _ => None,
        })
}

/// Page the owner's own messages of one Superagent chat. `chat_id` defaults to
/// the newest non-archived ticket's `superagent` origin in this project; an
/// explicit chat must have sent work here (otherwise the lead could read any
/// chat on the machine).
/// # Errors
/// Returns an error if the lead chat cannot be resolved or its session messages cannot be read.
#[allow(
    clippy::unused_async,
    reason = "Preserve the public async service API and execution-on-poll semantics for callers."
)]
pub async fn page(
    layout: &Layout,
    project: &Project,
    chat_id: Option<&str>,
    query: Option<&str>,
    before: Option<&str>,
    limit: usize,
    sessions_root: &Path,
) -> Result<Value, String> {
    let limit = limit.clamp(1, 50);
    let board = Board::open(layout, project.clone()).map_err(|e| e.to_string())?;
    let tasks = board.tasks().map_err(|e| e.to_string())?;
    let origins: Vec<(String, String)> = tasks
        .iter()
        .filter(|task| task.status != Status::Archived)
        .filter_map(|task| {
            task.origin
                .as_ref()
                .filter(|origin| origin.kind == OriginKind::Superagent)
                .and_then(|origin| origin.session.clone())
                .map(|chat| (chat, task.id.clone()))
        })
        .collect();
    let chat = match chat_id.map(str::trim).filter(|chat| !chat.is_empty()) {
        Some(chat) => {
            if !origins.iter().any(|(known, _)| known == chat) {
                return Err(format!(
                    "This chat sent no work to {}; you can read only chats named in your tickets' origin.",
                    project.slug
                ));
            }
            chat.to_owned()
        }
        None => origins
            .last()
            .map(|(chat, _)| chat.clone())
            .ok_or_else(|| "No Superagent chat has sent work to this project yet.".to_owned())?,
    };
    let file = session_file(&chat, project, sessions_root)?;
    let text =
        std::fs::read_to_string(&file).map_err(|e| format!("reading {}: {e}", file.display()))?;
    let parsed = oino_session::migrate::parse_session_text(&text)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{} is not a session file", file.display()))?;
    let blobs_dir =
        crate::sessions::SessionService::shared().map(|service| service.hub().blobs_dir());
    let messages: Vec<Value> = parsed
        .entries
        .iter()
        .filter_map(|entry| owner_message(entry, blobs_dir.as_deref()))
        .collect();
    let total = messages.len();
    // A query searches the whole set (best match first); a blank query keeps
    // today's newest-first paging byte for byte.
    let trimmed = query.map(str::trim).filter(|q| !q.is_empty());
    let Some(query) = trimmed else {
        let mut newest: Vec<&Value> = messages.iter().rev().collect();
        // `before` is the last id already seen: the page holds the messages
        // OLDER than it (exclusive), so the next page never repeats a message.
        if let Some(cursor) = before.map(str::trim).filter(|cursor| !cursor.is_empty()) {
            if let Some(position) = newest.iter().position(|m| m["id"] == json!(cursor)) {
                newest.drain(..position);
            }
        }
        let page: Vec<Value> = newest.iter().take(limit).map(|m| (*m).clone()).collect();
        let next = newest.get(limit).map_or(Value::Null, |m| m["id"].clone());
        return Ok(json!({"chatId": chat, "messages": page, "before": next, "total": total}));
    };
    let docs: Vec<&str> = messages.iter().filter_map(|m| m["text"].as_str()).collect();
    let scores = bm25_scores(query, &docs);
    let terms = query_terms(query);
    let mut hits: Vec<(usize, f64)> = scores
        .into_iter()
        .enumerate()
        .filter(|(_, score)| *score > 0.0)
        .collect();
    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    let matched = hits.len();
    let page: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|(i, score)| {
            let mut m = messages[i].clone();
            m["score"] = json!((score * 1_000.0).round() / 1_000.0);
            m["snippet"] = json!(snippet(docs[i], &terms));
            m
        })
        .collect();
    Ok(
        json!({"chatId": chat, "query": query, "messages": page, "before": Value::Null, "total": matched}),
    )
}

fn owner_message(entry: &oino_session::SessionEntry, blobs_dir: Option<&Path>) -> Option<Value> {
    match entry.message() {
        Some(AgentMessage::User(user)) if user.author.is_none() => {
            let text = match &user.content {
                UserContentField::Text(text) => text.clone(),
                UserContentField::Blocks(blocks) => blocks
                    .iter()
                    .map(|block| match block {
                        UserContent::Text(text) => text.text.clone(),
                        UserContent::Image(image) => owner_image_path(&image.data, blobs_dir)
                            .map_or_else(
                                || "[image]".to_owned(),
                                |path| format!("[image: {}]", path.display()),
                            ),
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            Some(json!({"id": entry.id.as_str(), "at": user.timestamp, "text": text}))
        }
        _ => None,
    }
}

/// Resolve an owner's inline or host-blob image to a readable, content-addressed file.
fn owner_image_path(data: &str, blobs_dir: Option<&Path>) -> Option<PathBuf> {
    use base64::Engine as _;
    let (bytes, ext) = if data.len() == 64 && data.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes = std::fs::read(blobs_dir?.join(data)).ok()?;
        let ext = image_extension(&bytes);
        (bytes, ext)
    } else {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?;
        let ext = image_extension(&bytes);
        (bytes, ext)
    };
    let digest = sha2::Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        });
    let dir = blobs_dir?.join("owner-images");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{digest}.{ext}"));
    if !path.exists() {
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

fn image_extension(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ => "bin",
    }
}

/// Stopwords dropped before ranking, the `session_recall` list
/// (extensions/unipi/compactor/src/recall.rs).
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "can", "shall", "of",
    "in", "to", "for", "with", "on", "at", "from", "by", "as", "into", "through", "during",
    "before", "after", "above", "below", "between", "out", "off", "over", "under", "again",
    "further", "then", "once", "here", "there", "when", "where", "why", "how", "all", "both",
    "each", "few", "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own",
    "same", "so", "than", "too", "very", "just", "about", "it", "its", "that", "this", "what",
    "which", "who", "whom", "these", "those",
];

/// The query split into ranked terms: whitespace words minus stopwords and
/// single characters, falling back to the raw words when nothing survives.
#[must_use]
fn query_terms(query: &str) -> Vec<&str> {
    let raw: Vec<&str> = query.split_whitespace().collect();
    let meaningful: Vec<&str> = raw
        .iter()
        .copied()
        .filter(|t| !STOPWORDS.contains(&t.to_lowercase().as_str()) && t.chars().count() > 1)
        .collect();
    if meaningful.is_empty() {
        raw
    } else {
        meaningful
    }
}

/// Non-overlapping case-insensitive occurrence count of `needle` in `hay`.
#[must_use]
fn count_matches(hay: &str, needle: &str) -> usize {
    let (hay, needle) = (hay.to_lowercase(), needle.to_lowercase());
    let mut count = 0usize;
    let mut rest = hay.as_str();
    while let Some(at) = rest.find(&needle) {
        count += 1;
        rest = &rest[at + needle.len()..];
    }
    count
}

/// BM25 scores per document (`session_recall`'s scorer: k1 1.2, b 0.75,
/// idf `ln((N - df + 0.5)/(df + 0.5) + 1)`). Documents matching no term are
/// absent from the result.
#[must_use]
fn bm25_scores(query: &str, docs: &[&str]) -> Vec<f64> {
    let terms = query_terms(query);
    if docs.is_empty() || terms.is_empty() {
        return vec![0.0; docs.len()];
    }
    // Document length in words, the corpus average, and per-term df.
    let lengths: Vec<usize> = docs
        .iter()
        .map(|d| d.split_whitespace().count().max(1))
        .collect();
    let n = docs.len();
    #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
    let avgdl = lengths.iter().sum::<usize>() as f64 / n as f64;
    let df: Vec<usize> = terms
        .iter()
        .map(|term| docs.iter().filter(|d| count_matches(d, term) > 0).count())
        .collect();
    docs.iter()
        .enumerate()
        .map(|(i, doc)| {
            #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
            let dl = lengths[i] as f64;
            let mut score = 0.0;
            for (ti, term) in terms.iter().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
                let tf = count_matches(doc, term) as f64;
                if tf == 0.0 {
                    continue;
                }
                #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
                let (dfv, nn) = (df[ti] as f64, n as f64);
                let idf = ((nn - dfv + 0.5) / (dfv + 0.5) + 1.0).ln();
                score += idf * (tf * 2.2) / (tf + 1.2 * (0.25 + 0.75 * dl / avgdl));
            }
            score
        })
        .collect()
}

/// One line around the first match of any query term, `…` on both sides.
#[must_use]
fn snippet(text: &str, terms: &[&str]) -> String {
    let lower = text.to_lowercase();
    let Some(at) = terms
        .iter()
        .filter_map(|t| lower.find(&t.to_lowercase()))
        .min()
    else {
        return text.lines().next().unwrap_or_default().to_owned();
    };
    let mut start = at.saturating_sub(60);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + 80).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(text[start..end].trim());
    if end < text.len() {
        out.push('…');
    }
    out
}

/// The chat's session file: the live runtime's, else a lookup by id across the
/// sessions root (parse takes no lease — a live chat is never disturbed).
fn session_file(chat: &str, project: &Project, sessions_root: &Path) -> Result<PathBuf, String> {
    if let Some(file) =
        crate::superagent_session::runtime(chat).and_then(|rt| rt.session().session_file())
    {
        return Ok(file);
    }
    oino_session::SessionManager::find_by_id(&project.root, chat, sessions_root)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Superagent chat {chat} not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_session::entry::{EntryId, EntryKind, IsoTimestamp, SessionEntry};
    use std::fmt::Write as _;
    use std::sync::Arc;
    use unipi_kanboard::{
        commands::{self, Common},
        model::{Actor, ChainGate, Origin, Priority},
    };

    fn fixture() -> (tempfile::TempDir, Layout, Project) {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::with_home(home.path().to_owned());
        let project = Project::create(&layout, root.path(), Some("test"), Some("T")).unwrap();
        (home, layout, project)
    }

    fn add_with_origin(layout: &Layout, project: &Project, chat: &str) -> String {
        let mut common = Common::new(Actor::User, ChainGate::InReview);
        common.origin = Some(Origin {
            kind: OriginKind::Superagent,
            session: Some(chat.into()),
            message: Some("m-entry-9".into()),
        });
        commands::add(
            layout,
            project.clone(),
            &common,
            "From the chat",
            Some("body"),
            Some(Status::Backlog),
            Priority::Medium,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }

    /// A v3 session file holding two owner turns (no author), one host-relay
    /// (author "superagent") and one custom context message between them.
    fn write_chat(dir: &Path, sessions_root: &Path) -> PathBuf {
        let entry = |id: &str, kind: EntryKind| SessionEntry {
            id: EntryId::new(id),
            parent_id: None,
            timestamp: IsoTimestamp::from_ms(1_000),
            compaction_summary_id: None,
            kind,
        };
        let user = |text: &str| EntryKind::Message {
            message: AgentMessage::User(oino_ai::UserMessage {
                author: None,
                content: UserContentField::Text(text.into()),
                timestamp: 5_000,
            }),
        };
        let entries = [
            entry(
                "e1",
                user("please fix the login timeout, it repros every night"),
            ),
            entry(
                "e2",
                EntryKind::Message {
                    message: AgentMessage::User(oino_ai::UserMessage {
                        author: Some("superagent".into()),
                        content: UserContentField::Text("relayed to the lead".into()),
                        timestamp: 5_001,
                    }),
                },
            ),
            entry(
                "e3",
                EntryKind::Custom {
                    custom_type: "oino:superagent-context".into(),
                    data: None,
                },
            ),
            entry(
                "e4",
                user("also the dashboard chart is blank after midnight"),
            ),
        ];
        let chat_dir = sessions_root.join("--home-oi-.oino-superagent");
        std::fs::create_dir_all(&chat_dir).unwrap();
        let mut text = format!("{{\"type\":\"session\",\"version\":3,\"id\":\"{}\",\"timestamp\":\"2026-11-11T08:00:00.000Z\",\"cwd\":\"{}\"}}\n", "chat-abc12345", dir.display());
        for e in entries {
            text.push_str(&serde_json::to_string(&e).unwrap());
            text.push('\n');
        }
        let file = chat_dir.join("20261111T080000-000Z_chat-abc12345.jsonl");
        std::fs::write(&file, text).unwrap();
        file
    }

    #[tokio::test]
    async fn pages_verbatim_newest_first_and_excludes_host() {
        let (home, layout, project) = fixture();
        let sessions_root = home.path().join("sessions");
        write_chat(home.path(), &sessions_root);
        let id = add_with_origin(&layout, &project, "chat-abc12345");
        assert!(id.starts_with("T-"));
        let payload = page(&layout, &project, None, None, None, 20, &sessions_root)
            .await
            .unwrap();
        assert_eq!(payload["chatId"], "chat-abc12345");
        assert_eq!(payload["total"], 2);
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        // Newest first, verbatim, untruncated; host/custom entries excluded.
        assert_eq!(messages[0]["id"], "e4");
        assert_eq!(
            messages[0]["text"],
            "also the dashboard chart is blank after midnight"
        );
        assert_eq!(messages[0]["at"], 5_000);
        assert_eq!(messages[1]["id"], "e1");
        assert!(messages[1]["text"]
            .as_str()
            .unwrap()
            .contains("every night"));
        // Paging: limit 1, then the cursor from that page returns the older
        // message only; with the whole page consumed the cursor is null.
        let first = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            None,
            None,
            1,
            &sessions_root,
        )
        .await
        .unwrap();
        assert_eq!(first["messages"].as_array().unwrap().len(), 1);
        assert_eq!(first["messages"][0]["id"], "e4");
        let cursor = first["before"].as_str().unwrap();
        let second = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            None,
            Some(cursor),
            20,
            &sessions_root,
        )
        .await
        .unwrap();
        let rest = second["messages"].as_array().unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0]["id"], "e1");
        assert!(second["before"].is_null());
        // An explicit chatId that sent work here is allowed.
        let explicit = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            None,
            None,
            1,
            &sessions_root,
        )
        .await
        .unwrap();
        assert_eq!(explicit["messages"].as_array().unwrap().len(), 1);
        assert_eq!(explicit["total"], 2);
    }

    #[tokio::test]
    async fn query_searches_owner_messages_and_still_denies_unrelated() {
        let (home, layout, project) = fixture();
        let sessions_root = home.path().join("sessions");
        write_chat(home.path(), &sessions_root);
        add_with_origin(&layout, &project, "chat-abc12345");
        // "dashboard" matches only the newest owner message; the host relay
        // ("relayed to the lead") and the custom context entry never qualify.
        let payload = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            Some("  dashboard chart  "),
            None,
            20,
            &sessions_root,
        )
        .await
        .unwrap();
        assert_eq!(payload["chatId"], "chat-abc12345");
        assert_eq!(payload["query"], "dashboard chart");
        assert_eq!(payload["total"], 1);
        assert!(payload["before"].is_null(), "query mode has no cursor");
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], "e4");
        assert!(messages[0]["score"].as_f64().is_some_and(|s| s > 0.0));
        assert_eq!(
            messages[0]["snippet"],
            "also the dashboard chart is blank after midnight"
        );
        assert!(messages.iter().all(|m| m["text"] != "relayed to the lead"));
        // Ranking: a query matching both messages returns both, best score
        // first (unit-level ordering is pinned in bm25_orders_docs below).
        let both = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            Some("every midnight"),
            None,
            20,
            &sessions_root,
        )
        .await
        .unwrap();
        let both = both["messages"].as_array().unwrap();
        assert_eq!(both.len(), 2);
        assert!(both.iter().any(|m| m["id"] == "e1"));
        assert!(both.iter().any(|m| m["id"] == "e4"));
        assert!(both[0]["score"].as_f64().unwrap() >= both[1]["score"].as_f64().unwrap());
        // No matches: empty page, zero total, no cursor.
        let none = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            Some("zzzunmatched"),
            None,
            20,
            &sessions_root,
        )
        .await
        .unwrap();
        assert_eq!(none["messages"].as_array().unwrap().len(), 0);
        assert_eq!(none["total"], 0);
        assert!(none["before"].is_null());
        // A blank query behaves exactly like paging (no query key, cursor set).
        let blank = page(
            &layout,
            &project,
            Some("chat-abc12345"),
            Some("   "),
            None,
            1,
            &sessions_root,
        )
        .await
        .unwrap();
        assert!(blank.get("query").is_none());
        assert_eq!(blank["messages"].as_array().unwrap().len(), 1);
        assert_eq!(blank["messages"][0]["id"], "e4");
        assert!(blank["before"].is_string());
        // The access check runs before any search: an unrelated chat is refused
        // even with a query.
        let denied = page(
            &layout,
            &project,
            Some("chat-other"),
            Some("dashboard"),
            None,
            20,
            &sessions_root,
        )
        .await;
        assert!(denied.unwrap_err().contains("sent no work to"));
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
    }

    #[test]
    fn writes_inline_owner_image_once_to_readable_content_addressed_path() {
        use base64::Engine as _;
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let path = owner_image_path(&encoded, Some(dir.path())).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(path, owner_image_path(&encoded, Some(dir.path())).unwrap());
        assert!(path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn query_terms_drops_stopwords_and_single_chars() {
        assert_eq!(
            query_terms("fix the login timeout"),
            vec!["fix", "login", "timeout"]
        );
        // Everything filtered: fall back to the raw words.
        assert_eq!(query_terms("the is of a"), vec!["the", "is", "of", "a"]);
        assert_eq!(query_terms("  "), Vec::<&str>::new());
        assert_eq!(query_terms("x y repro"), vec!["repro"]);
    }

    #[test]
    fn count_matches_is_case_insensitive_and_non_overlapping() {
        assert_eq!(count_matches("Timeout TIMEOUT timeout", "timeout"), 3);
        assert_eq!(count_matches("aaaa", "aa"), 2);
        assert_eq!(count_matches("nothing here", "zzz"), 0);
    }

    #[test]
    fn bm25_orders_docs_and_skips_non_matches() {
        let docs = [
            "fix the login timeout, it repros every night",
            "unrelated chatter about the weather",
            "login broken, login slow, login gone",
            "the dashboard chart is blank",
        ];
        let scores = bm25_scores("login timeout", &docs);
        // Only matching docs score above zero.
        assert_eq!(scores[1], 0.0);
        assert_eq!(scores[3], 0.0);
        // doc 2 repeats "login" three times, doc 0 has both terms.
        assert!(scores[0] > 0.0);
        assert!(scores[2] > 0.0);
        let mut order: Vec<usize> = (0..docs.len()).filter(|i| scores[*i] > 0.0).collect();
        order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
        // The tf term saturates, so the rarer term's idf dominates: the doc
        // holding BOTH terms (login + the df=1 "timeout") beats 3x "login".
        assert_eq!(order, vec![0, 2]);
        // No matching term at all: every score is zero.
        assert!(bm25_scores("zzzunmatched", &docs).iter().all(|s| *s == 0.0));
        // Case-insensitive: upper-case query matches lower-case doc.
        assert!(bm25_scores("LOGIN", &docs)[2] > 0.0);
    }

    #[test]
    fn snippet_centers_on_first_match() {
        assert_eq!(snippet("blank chart", &["chart"]), "blank chart");
        let long = format!("{} dashboard {}", "x".repeat(100), "y".repeat(100));
        let s = snippet(&long, &["dashboard"]);
        assert!(s.starts_with('…'), "{s}");
        assert!(s.ends_with('…'), "{s}");
        assert!(s.contains("dashboard"), "{s}");
        // Multi-byte characters are never split mid-char.
        let umlauts = format!("{} über dashboard", "ä".repeat(80));
        assert!(snippet(&umlauts, &["dashboard"]).contains("über dashboard"));
        // No match: fall back to the first line.
        assert_eq!(snippet("first line\nsecond", &["zzz"]), "first line");
    }

    #[tokio::test]
    async fn denies_unrelated_chat_and_empty_project() {
        let (home, layout, project) = fixture();
        let sessions_root = home.path().join("sessions");
        add_with_origin(&layout, &project, "chat-known");
        let unrelated = page(
            &layout,
            &project,
            Some("chat-other"),
            None,
            None,
            20,
            &sessions_root,
        )
        .await;
        assert!(unrelated.unwrap_err().contains("sent no work to"));
        let empty_project = {
            let (_, _, other) = fixture();
            page(&layout, &other, None, None, None, 20, &sessions_root).await
        };
        assert!(empty_project
            .unwrap_err()
            .contains("No Superagent chat has sent work"));
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
    }

    /// "Send as note": an `oino.note` custom entry (with an image) between owner turns is
    /// never an owner message — `owner_messages` neither lists it nor finds it by query.
    #[tokio::test]
    async fn owner_messages_ignore_send_as_note_entries() {
        let (home, layout, project) = fixture();
        let sessions_root = home.path().join("sessions");
        let file = write_chat(home.path(), &sessions_root);
        let note = serde_json::json!({
            "type": "custom", "id": "n1", "parentId": null, "timestamp": "2026-11-11T08:00:02.000Z",
            "customType": "oino.note",
            "data": {"text": "private reminder about quasar", "images": [{"type": "image", "data": "aGk=", "mimeType": "image/png"}]},
        });
        let mut text = std::fs::read_to_string(&file).unwrap();
        let _ = writeln!(text, "{note}");
        std::fs::write(&file, text).unwrap();
        add_with_origin(&layout, &project, "chat-abc12345");
        let all = page(&layout, &project, None, None, None, 20, &sessions_root)
            .await
            .unwrap();
        assert_eq!(all["total"], 2, "the note is not an owner message");
        assert!(all["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"] != "n1"));
        let found = page(
            &layout,
            &project,
            None,
            Some("quasar"),
            None,
            20,
            &sessions_root,
        )
        .await
        .unwrap();
        assert_eq!(
            found["total"], 0,
            "a note is not searchable through owner_messages"
        );
    }

    /// The image handoff to a new ticket takes the latest real owner turn's images; a
    /// later note's images are never attached, and a note is not the origin-stamp entry.
    #[test]
    fn note_images_are_not_handed_to_tickets_and_do_not_move_the_origin() {
        use oino_core::runtime::{
            AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection,
        };
        use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let runtime = rt.block_on(async {
            let services = Services::create(ServicesOptions {
                cwd: dir.path().into(),
                agent_dir: dir.path().into(),
                docs_root: PathBuf::new(),
                mode: oino_core::extensions::api::ExtensionMode::Rpc,
                has_ui: false,
                flags: ResourceFlags {
                    no_context_files: true,
                    ..Default::default()
                },
                extension_flag_values: std::iter::empty().collect(),
                loader: Arc::new(StaticLoader(Vec::new())),
                approve: Some(false),
                models: None,
                settings: None,
            })
            .await
            .unwrap();
            AgentSessionRuntime::create_for_hub(
                services,
                SessionOptions {
                    source: SessionSource::New,
                    tools: ToolSelection::NoBuiltin,
                    session_dir: Some(dir.path().to_path_buf()),
                    ..SessionOptions::default()
                },
            )
            .await
            .unwrap()
        });
        let runtime = Arc::new(runtime);
        let sid = runtime.session().session_id();
        crate::superagent_session::register(Arc::clone(&runtime));
        let png = |data: &str| oino_ai::ImageContent {
            data: data.into(),
            mime_type: "image/png".into(),
        };
        let owner = oino_ai::UserMessage {
            author: None,
            content: UserContentField::Blocks(vec![
                UserContent::Text(oino_ai::TextContent {
                    text: "see screenshot".into(),
                    text_signature: None,
                }),
                UserContent::Image(png("b3duZXI=")),
            ]),
            timestamp: 1,
        };
        let agent = runtime.session();
        agent
            .with_manager(|m| {
                m.append_message(AgentMessage::User(owner))?;
                m.materialize()
            })
            .unwrap();
        let owner_entry = latest_owner_entry_from(&sid).unwrap();
        agent
            .append_note("my own note", &[png("bm90ZQ==")])
            .unwrap();
        let images = latest_owner_images_from(&sid);
        assert_eq!(
            images.len(),
            1,
            "only the owner turn's image is handed over"
        );
        assert_eq!(images[0].1, b"owner");
        assert_eq!(
            latest_owner_entry_from(&sid),
            Some(owner_entry),
            "a note never becomes the origin entry"
        );
        crate::superagent_session::unregister(&sid);
    }

    #[test]
    fn latest_owner_entry_skips_authored_and_custom_entries() {
        use oino_core::runtime::{
            AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection,
        };
        use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let runtime = rt.block_on(async {
            let services = Services::create(ServicesOptions {
                cwd: dir.path().into(),
                agent_dir: dir.path().into(),
                docs_root: PathBuf::new(),
                mode: oino_core::extensions::api::ExtensionMode::Rpc,
                has_ui: false,
                flags: ResourceFlags {
                    no_context_files: true,
                    ..Default::default()
                },
                extension_flag_values: std::iter::empty().collect(),
                loader: Arc::new(StaticLoader(Vec::new())),
                approve: Some(false),
                models: None,
                settings: None,
            })
            .await
            .unwrap();
            AgentSessionRuntime::create_for_hub(
                services,
                SessionOptions {
                    source: SessionSource::New,
                    tools: ToolSelection::NoBuiltin,
                    session_dir: Some(dir.path().to_path_buf()),
                    ..SessionOptions::default()
                },
            )
            .await
            .unwrap()
        });
        let agent = runtime.session();
        agent
            .with_manager(|m| {
                m.append_message(AgentMessage::User(oino_ai::UserMessage {
                    author: None,
                    content: UserContentField::Text("first".into()),
                    timestamp: 1,
                }))?;
                m.append_custom_message_entry(
                    "oino:superagent-context",
                    oino_ai::UserContentField::Text("context".into()),
                    false,
                    Some(json!({"author":"host"})),
                )?;
                m.append_message(AgentMessage::User(oino_ai::UserMessage {
                    author: Some("superagent".into()),
                    content: UserContentField::Text("relay".into()),
                    timestamp: 2,
                }))?;
                m.append_message(AgentMessage::User(oino_ai::UserMessage {
                    author: None,
                    content: UserContentField::Text("second".into()),
                    timestamp: 3,
                }))?;
                m.materialize()
            })
            .unwrap();
        // The branch's last owner turn wins; authored and custom entries never qualify.
        let latest = latest_owner_entry(&agent);
        assert_ne!(latest, None);
        let latest = latest.unwrap();
        let expected = agent.with_manager(|m| {
            m.branch(None).iter().rev().find_map(|e| match e.message() {
                Some(AgentMessage::User(u))
                    if u.author.is_none() && u.content.text(" ") == "second" =>
                {
                    Some(e.id.to_string())
                }
                _ => None,
            })
        });
        assert_eq!(Some(latest), expected);
    }
}
