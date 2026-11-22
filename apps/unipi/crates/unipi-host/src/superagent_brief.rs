//! The lead work brief (S0 split from `superagent.rs`; OIN-83 layout).
//!
//! Preset values travel in each brief (never in the frozen system prompt), so a
//! changed preset never invalidates a lead's prompt cache.

use unipi_kanboard::model::OriginKind;
use unipi_kanboard::{model::Task, Project};

/// `deadbeef` — first 8 chars of a session id in origin/brief lines.
fn s8(session: &str) -> &str {
    &session[..session
        .char_indices()
        .nth(8)
        .map_or(session.len(), |(i, _)| i)]
}

/// One line describing where a ticket came from (OIN-83).
#[must_use]
pub(crate) fn origin_line(task: &Task) -> String {
    let Some(origin) = &task.origin else {
        return "not recorded (created before origin tracking)".into();
    };
    match origin.kind {
        OriginKind::Superagent => {
            let Some(chat) = origin.session.as_deref().filter(|chat| !chat.is_empty()) else {
                return "not recorded (created before origin tracking)".into();
            };
            let message = origin.message.as_deref().unwrap_or("-");
            format!(
                "owner message {m8} in Superagent chat {c8}; read it word for word with owner_messages {{\"chatId\":\"{c}\"}}",
                m8 = s8(message),
                c8 = s8(chat),
                c = chat,
            )
        }
        OriginKind::Owner => "added by the owner".into(),
        _ => {
            let Some(session) = origin.session.as_deref().filter(|s| !s.is_empty()) else {
                return "not recorded (created before origin tracking)".into();
            };
            format!("created by {} session {}", origin.kind, s8(session))
        }
    }
}

/// The preset line travels in the brief (never the frozen system prompt), so a
/// changed preset never changes the lead's prompt.
pub(crate) fn lead_brief(
    project: &Project,
    tasks: &[Task],
    preset: &crate::superagent_presets::Preset,
) -> String {
    let preset_line = format!(
        "Preset for this batch: {label}: mode {mode}, at most {parallel} tickets in parallel, review {review}{profiles}.\n\n",
        label = if preset.label.is_empty() { "default" } else { preset.label.as_str() },
        mode = preset.mode,
        parallel = preset.max_parallel.unwrap_or(1),
        review = preset.review,
        profiles = preset.profiles.as_ref().map_or_else(String::new, |profiles| format!("; subagent profiles: {}", profiles.join(", "))),
    );
    let tickets = tasks
        .iter()
        .map(|task| {
            let incomplete = task
                .activity
                .iter()
                .rev()
                .find(|a| {
                    a.text
                        .starts_with(unipi_kanboard::dispatcher::RELEASED_INCOMPLETE_PREFIX)
                })
                .map(|a| format!("{}\n", a.text))
                .unwrap_or_default();
            let context = task
                .activity
                .iter()
                .rev()
                .find(|a| a.text.starts_with("Owner answer:"))
                .map(|answer| {
                    let question = task
                        .activity
                        .iter()
                        .rev()
                        .find(|a| a.text.starts_with("blocked:"))
                        .map_or("", |a| a.text.as_str());
                    format!("\n{question}\n{}", answer.text)
                })
                .unwrap_or_default();
            format!(
                "## {id} — {title}\nOrigin: {origin}\n{incomplete}{body}{context}",
                id = task.id,
                title = task.title,
                origin = origin_line(task),
                body = task.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("Lead work for project {slug}. {preset_line}Claim each ticket with your kanboard tool, work them using your mode to parallelise where appropriate, and finish each to in_review with a concise high-level report in simple technical English (unless the owner asks for detail), or to blocked with a question. Never leave a ticket in_progress. For visible results, attach a screenshot to the ticket with finish --attach <png> or attach <ID> <png> --note <what it shows>; optional if there is no visual result or screenshot. Do not only paste screenshots in chat. For ticket images, use show <ID> --json or attachments <ID> to get absolute paths, then read the image path if your model supports vision; otherwise use image_recognize. If a ticket cites an owner message in Superagent chat, read that exact message with owner_messages too; those original messages may contain the only copy of an image not attached to the ticket. Never silently ignore an available image.\n\n{tickets}", slug = project.slug)
}

#[cfg(test)]
mod lead_tests {
    #[test]
    fn brief_keeps_full_body_and_names_origin() {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let layout = unipi_kanboard::Layout::with_home(home.path().to_owned());
        let project =
            unipi_kanboard::Project::create(&layout, root.path(), Some("test"), Some("T")).unwrap();
        let body = "x".repeat(5_000);
        let mut common_chat = unipi_kanboard::commands::Common::new(
            unipi_kanboard::model::Actor::User,
            unipi_kanboard::model::ChainGate::InReview,
        );
        common_chat.origin = Some(unipi_kanboard::model::Origin {
            kind: unipi_kanboard::model::OriginKind::Superagent,
            session: Some("chat-abcd1234".into()),
            message: Some("m-1234ef56".into()),
        });
        let chat_ticket = unipi_kanboard::commands::add(
            &layout,
            project.clone(),
            &common_chat,
            "From chat",
            Some(body.as_str()),
            Some(unipi_kanboard::model::Status::Todo),
            unipi_kanboard::model::Priority::Medium,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap();
        let mut common_owner = unipi_kanboard::commands::Common::new(
            unipi_kanboard::model::Actor::User,
            unipi_kanboard::model::ChainGate::InReview,
        );
        common_owner.origin = Some(unipi_kanboard::model::Origin {
            kind: unipi_kanboard::model::OriginKind::Owner,
            session: None,
            message: None,
        });
        let plain = unipi_kanboard::commands::add(
            &layout,
            project.clone(),
            &common_owner,
            "Plain",
            Some("Body A"),
            Some(unipi_kanboard::model::Status::Todo),
            unipi_kanboard::model::Priority::Medium,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap();
        let board = unipi_kanboard::board::Board::open(&layout, project.clone()).unwrap();
        let tasks = [
            chat_ticket["id"].as_str().unwrap(),
            plain["id"].as_str().unwrap(),
        ]
        .iter()
        .map(|id| board.get(id).unwrap())
        .collect::<Vec<_>>();
        let preset = crate::superagent_presets::Store::default()
            .resolve(
                Some("feature"),
                crate::superagent_presets::Overrides::default(),
            )
            .unwrap();
        let brief = super::lead_brief(&project, &tasks, &preset);
        // The preset line carries maxParallel, review and (none here) profiles.
        assert!(brief.contains("Preset for this batch: Feature: mode graph, at most 3 tickets in parallel, review full."), "{brief}");
        // The 5,000-char body survives whole.
        assert!(brief.contains(&body), "body truncated");
        assert!(brief.contains("## T-1 — From chat"), "{brief}");
        assert!(brief.contains("## T-2 — Plain"), "{brief}");
        assert!(brief.contains("Body A"), "{brief}");
        for guidance in [
            "concise high-level report",
            "simple technical English",
            "finish --attach <png>",
            "attachments <ID>",
            "image_recognize",
            "Never silently ignore an available image",
            "read that exact message with owner_messages",
        ] {
            assert!(brief.contains(guidance), "missing {guidance}: {brief}");
        }
        // The origin names the chat, the message and the tool call.
        assert!(brief.contains("owner message m-1234ef in Superagent chat chat-abc; read it word for word with owner_messages {\"chatId\":\"chat-abcd1234\"}"), "{brief}");
        // The owner-added ticket says so; instructions stay for the existing test.
        assert!(brief.contains("added by the owner"), "{brief}");
        for text in ["Claim each ticket", "parallelise", "in_review", "blocked"] {
            assert!(brief.contains(text), "missing {text}");
        }
    }

    #[test]
    fn brief_renders_latest_released_incomplete_line_verbatim_first_under_ticket() {
        let now = chrono::Utc::now();
        let mut task = unipi_kanboard::model::Task::new(
            "T-7".into(),
            "Settle incomplete".into(),
            unipi_kanboard::model::Status::InProgress,
            unipi_kanboard::model::Priority::High,
            0,
            now,
        );
        task.body = "Full ticket body after the incomplete notice.".into();
        let older = unipi_kanboard::dispatcher::released_incomplete_line(
            unipi_kanboard::model::Status::InProgress,
            "session-older-1111",
            "session process gone",
            None,
        );
        let latest = unipi_kanboard::dispatcher::released_incomplete_line(
            unipi_kanboard::model::Status::Blocked,
            "session-latest-2222",
            "lead stopped; moved to backlog so Auto-start does not relaunch it; move to todo to run again",
            Some("Which storage engine?"),
        );
        task.push_activity(now, unipi_kanboard::model::Actor::System, older.clone());
        task.push_activity(now, unipi_kanboard::model::Actor::System, latest.clone());

        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let layout = unipi_kanboard::Layout::with_home(home.path().to_owned());
        let project =
            unipi_kanboard::Project::create(&layout, root.path(), Some("test"), Some("T")).unwrap();
        let preset = crate::superagent_presets::Store::default()
            .resolve(
                Some("feature"),
                crate::superagent_presets::Overrides::default(),
            )
            .unwrap();

        let brief = super::lead_brief(&project, &[task], &preset);
        assert!(
            brief.contains(&latest),
            "missing latest line in brief:\n{brief}"
        );
        assert!(
            !brief.contains(&older),
            "older incomplete line must not be duplicated:\n{brief}"
        );
        let latest_pos = brief.find(&latest).unwrap();
        let body_pos = brief
            .find("Full ticket body after the incomplete notice.")
            .unwrap();
        assert!(
            latest_pos < body_pos,
            "Released incomplete line must appear before the ticket body so the lead sees it first:\n{brief}"
        );
    }

    #[test]
    fn origin_line_covers_all_kinds_and_legacy_tickets() {
        let origin =
            |kind, session: Option<&str>, message: Option<&str>| unipi_kanboard::model::Origin {
                kind,
                session: session.map(str::to_owned),
                message: message.map(str::to_owned),
            };
        let mut task = unipi_kanboard::model::Task::new(
            "T-1".into(),
            "t".into(),
            unipi_kanboard::model::Status::Todo,
            unipi_kanboard::model::Priority::None,
            0,
            chrono::Utc::now(),
        );
        assert_eq!(
            super::origin_line(&task),
            "not recorded (created before origin tracking)"
        );
        task.origin = Some(origin(unipi_kanboard::model::OriginKind::Owner, None, None));
        assert_eq!(super::origin_line(&task), "added by the owner");
        task.origin = Some(origin(
            unipi_kanboard::model::OriginKind::Lead,
            Some("018f3f2a-aa11-7111-9c22-5f4cf9a1bbcd"),
            None,
        ));
        assert_eq!(
            super::origin_line(&task),
            "created by lead session 018f3f2a"
        );
        task.origin = Some(origin(
            unipi_kanboard::model::OriginKind::Superagent,
            Some("chat-abcd1234"),
            None,
        ));
        assert!(super::origin_line(&task).contains("{\"chatId\":\"chat-abcd1234\"}"));
    }

    #[test]
    fn long_horizon_owner_is_scoped_to_session() {
        let agent = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let path = agent
            .path()
            .join("state/unipi.long-horizon/workspace")
            .join(oino_wasm_host::workspace_id(
                &cwd.path().canonicalize().unwrap(),
            ))
            .join("state.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for kind in ["graph", "swarm", "goal", "ralph"] {
            for status in ["active", "parked"] {
                std::fs::write(
                    &path,
                    serde_json::to_vec(&serde_json::json!({status:{"kind":kind,"session":"lead"}}))
                        .unwrap(),
                )
                .unwrap();
                assert!(crate::superagent_workers::has_long_horizon_owner(
                    agent.path(),
                    cwd.path(),
                    "lead"
                ));
                assert!(!crate::superagent_workers::has_long_horizon_owner(
                    agent.path(),
                    cwd.path(),
                    "other"
                ));
            }
        }
    }
}
