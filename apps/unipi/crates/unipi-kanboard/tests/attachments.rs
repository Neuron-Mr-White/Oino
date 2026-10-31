//! Attachments: CLI attach + list and the `attachments` field on task JSON
//! (the HTTP upload/serve tests went with the daemon, round 5).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs,
    unreachable_pub,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::format_push_string,
    clippy::used_underscore_binding,
    clippy::needless_continue,
    reason = "integration tests: helpers outside #[test] fns may panic on fixture failures"
)]

mod common;

use common::{Fixture, cli};
use unipi_kanboard::model::{Priority, Status};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[test]
fn cli_attach_stores_the_file_and_logs_a_comment_that_embeds_it() {
    let fixture = Fixture::new();
    let task = fixture.add_with("with a screenshot", Status::Todo, Priority::None, &[]);
    let file = fixture.root().join("Screen Shot 1.png");
    std::fs::write(&file, PNG).unwrap();

    let out = cli(
        &fixture,
        &[
            "attach",
            &task.id,
            file.to_str().unwrap(),
            "--note",
            "the broken header",
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let attachment = &payload["attachment"];
    assert_eq!(attachment["kind"], "image");
    assert_eq!(attachment["mime"], "image/png");
    assert_eq!(attachment["original"], "Screen-Shot-1.png");
    let reference = attachment["ref"].as_str().unwrap();
    assert!(
        reference.starts_with(&format!("att:{}/", task.id)),
        "{reference}"
    );
    assert_eq!(
        attachment["markdown"],
        format!("![Screen-Shot-1.png]({reference})")
    );
    assert_eq!(
        std::fs::read(attachment["path"].as_str().unwrap()).unwrap(),
        PNG
    );

    // The comment carries the note and the embed, and survives the file round-trip.
    let last = payload["activity"].as_array().unwrap().last().unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        last,
        format!("the broken header\n![Screen-Shot-1.png]({reference})")
    );
    let reread = fixture
        .tasks()
        .into_iter()
        .find(|t| t.id == task.id)
        .unwrap();
    assert_eq!(reread.activity.last().unwrap().text, last);
    assert_eq!(payload["attachments"].as_array().unwrap().len(), 1);

    // Attaching identical bytes again reuses the stored file.
    let again = cli(
        &fixture,
        &["attach", &task.id, file.to_str().unwrap(), "--json"],
    );
    assert!(again.status.success());
    let listed = cli(&fixture, &["attachments", &task.id, "--json"]);
    let items: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(
        items.as_array().unwrap().len(),
        1,
        "same content is stored once"
    );

    // validate stays clean with a multi-line attachment comment.
    let validate = cli(&fixture, &["validate"]);
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stdout)
    );
}
