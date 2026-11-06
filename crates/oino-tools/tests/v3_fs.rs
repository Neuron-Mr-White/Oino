//! Decision B acceptance tests. Old transcript fixtures belong to wp-cache.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "acceptance assertions"
)]

use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{JsonValue, UserContent};
use oino_tools::{SessionEnv, ToolConfig, ToolContext, tool_by_name};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct Model(&'static str);
impl ToolContext for Model {
    fn session_env(&self) -> Option<SessionEnv> {
        Some(SessionEnv {
            session_id: "fs-test".to_owned(),
            model: Some(self.0.to_owned()),
            ..SessionEnv::default()
        })
    }
}

fn config(root: &std::path::Path) -> ToolConfig {
    let mut config = ToolConfig::new(root.to_string_lossy(), root.join("agent").to_string_lossy());
    config.context = Arc::new(Model("gpt-5"));
    config
}

async fn run(
    config: &ToolConfig,
    name: &str,
    args: JsonValue,
) -> Result<AgentToolResult, oino_agent::ToolError> {
    tool_by_name(name, config.clone())
        .unwrap()
        .execute(
            ToolInvocation {
                tool_call_id: "fs-test".to_owned(),
                args: args.as_object().unwrap().clone(),
                parent_tool_call_id: None,
            },
            UpdateSink::discard(),
            CancellationToken::new(),
        )
        .await
}
fn text(result: &AgentToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| {
            if let UserContent::Text(text) = content {
                Some(text.text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn schema_and_paging() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    for name in ["read", "write", "edit", "ls", "glob", "grep", "apply_patch"] {
        let tool = tool_by_name(name, config.clone()).unwrap();
        assert_eq!(
            tool.meta().parameters["additionalProperties"],
            false,
            "{name}"
        );
    }
    let schema = &tool_by_name("read", config.clone())
        .unwrap()
        .meta()
        .parameters
        .clone();
    assert_eq!(schema["required"], json!(["file_path"]));
    assert_eq!(schema["properties"]["limit"]["default"], 2400);
    assert!(schema["properties"].get("path").is_none());
    assert!(
        tool_by_name("grep", config.clone())
            .unwrap()
            .meta()
            .parameters["properties"]
            .get("search_mode")
            .is_none()
    );
    assert!(
        !oino_tools::builtin_tools(&config)
            .iter()
            .any(|tool| tool.meta().name == "find")
    );
    let path = root.path().join("long.rs");
    std::fs::write(
        &path,
        format!("fn example() {{\n{}\n}}\n", "body\n".repeat(2500)),
    )
    .unwrap();
    let bare = run(&config, "read", json!({"file_path": path}))
        .await
        .unwrap();
    assert!(text(&bare).contains("1→fn example()"));
    assert!(!text(&bare).contains("body".repeat(3).as_str()));
    let page = run(
        &config,
        "read",
        json!({"file_path":path,"offset":2,"limit":2,"line_numbers":true}),
    )
    .await
    .unwrap();
    assert!(text(&page).starts_with("2→body\n3→body"));
    assert!(text(&page).contains("offset=4"));
    let page = run(
        &config,
        "read",
        json!({"file_path":path,"offset":2,"limit":3000}),
    )
    .await
    .unwrap();
    assert!(text(&page).contains("offset=2402"));
    assert!(
        run(&config, "read", json!({"file_path":"relative"}))
            .await
            .is_err()
    );
    assert!(
        run(&config, "read", json!({"file_path":path,"offset":0}))
            .await
            .is_err()
    );
    std::fs::write(root.path().join("a.txt"), "Alpha\nbeta\nalpha\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "Alpha\n").unwrap();
    let glob = run(
        &config,
        "glob",
        json!({"patterns":["*.txt","*.rs"],"excludePatterns":["b.txt"]}),
    )
    .await
    .unwrap();
    assert!(text(&glob).contains("a.txt"));
    assert!(!text(&glob).contains("b.txt"));
    let ls = run(&config, "ls", json!({"ignorePatterns":["*.rs"]}))
        .await
        .unwrap();
    assert!(!text(&ls).contains("long.rs"));
    let grep = run(
        &config,
        "grep",
        json!({"pattern":"alpha","case_insensitive":true,"glob_pattern":"*.txt"}),
    )
    .await
    .unwrap();
    assert_eq!(text(&grep), "a.txt\nb.txt");
    let count = run(
        &config,
        "grep",
        json!({"pattern":"alpha","case_insensitive":true,"output_mode":"count"}),
    )
    .await
    .unwrap();
    assert!(text(&count).contains("a.txt:2"));
    let content = run(
        &config,
        "grep",
        json!({"pattern":"beta","output_mode":"content","context_before":1,"line_numbers":true}),
    )
    .await
    .unwrap();
    assert!(text(&content).contains("a.txt-1- Alpha"));
    assert!(text(&content).contains("a.txt:2: beta"));
    assert!(!text(&content).contains("a.txt-3-"));
    let multiline = run(
        &config,
        "grep",
        json!({"pattern":"Alpha\\nbeta","multiline":true,"output_mode":"content"}),
    )
    .await
    .unwrap();
    assert!(text(&multiline).contains("beta"));
    let path = root.path().join("huge.png");
    let mut image = vec![0; 5 * 1024 * 1024 + 1];
    image[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    image[8..12].copy_from_slice(&13_u32.to_be_bytes());
    image[12..16].copy_from_slice(b"IHDR");
    std::fs::write(&path, image).unwrap();
    assert!(
        run(&config, "read", json!({"file_path":path}))
            .await
            .unwrap_err()
            .to_string()
            .contains("5 MB")
    );
    let path = root.path().join("huge.pdf");
    let mut pdf = vec![0; 3 * 1024 * 1024 + 1];
    pdf[..5].copy_from_slice(b"%PDF-");
    std::fs::write(&path, pdf).unwrap();
    assert!(
        run(&config, "read", json!({"file_path":path}))
            .await
            .unwrap_err()
            .to_string()
            .contains("3 MB")
    );
}

#[tokio::test]
async fn atomic_write_and_edit() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let path = root.path().join("file.txt");
    std::fs::write(&path, b"\xef\xbb\xbfone\r\none\r\n").unwrap();
    run(&config, "read", json!({"file_path":path}))
        .await
        .unwrap();
    assert!(
        run(
            &config,
            "edit",
            json!({"file_path":path,"edits":[{"oldText":"one","newText":"two"}]})
        )
        .await
        .is_err()
    );
    let edited = run(
        &config,
        "edit",
        json!({"file_path":path,"edits":[{"oldText":"one","newText":"two","change_all":true}]}),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbftwo\r\ntwo\r\n");
    assert!(edited.details.unwrap().get("diff").is_some());
    std::fs::write(&path, b"external\r\n").unwrap();
    assert!(
        run(
            &config,
            "edit",
            json!({"file_path":path,"edits":[{"oldText":"external","newText":"changed"}]})
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("modified externally")
    );
    run(&config, "read", json!({"file_path":path}))
        .await
        .unwrap();
    run(
        &config,
        "edit",
        json!({"file_path":path,"edits":[{"oldText":"external","newText":"changed"}]}),
    )
    .await
    .unwrap();
    let untouched = root.path().join("untouched");
    std::fs::write(&untouched, "a\n").unwrap();
    run(
        &config,
        "edit",
        json!({"file_path":untouched,"edits":[{"oldText":"a","newText":"b"}]}),
    )
    .await
    .unwrap();
    assert!(run(&config,"edit",json!({"file_path":untouched,"edits":[{"oldText":"b","newText":"x"},{"oldText":"b","newText":"y"}]})).await.is_err());
    for little in [true, false] {
        let bytes: Vec<u8> = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        }
        .into_iter()
        .chain("old\r\n".encode_utf16().flat_map(|unit| {
            if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            }
        }))
        .collect();
        std::fs::write(&path, &bytes).unwrap();
        run(
            &config,
            "write",
            json!({"file_path":path,"content":"new\nnext"}),
        )
        .await
        .unwrap();
        let expected: Vec<u8> = bytes[..2]
            .iter()
            .copied()
            .chain("new\r\nnext\r\n".encode_utf16().flat_map(|unit| {
                if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            }))
            .collect();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        run(&config, "read", json!({"file_path":path}))
            .await
            .unwrap();
        run(
            &config,
            "edit",
            json!({"file_path":path,"edits":[{"oldText":"new","newText":"edited"}]}),
        )
        .await
        .unwrap();
    }
    let error = run(
        &config,
        "write",
        json!({"file_path":root.path(),"content":"x"}),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("because a directory already exists there.")
    );
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    let old = json!({"path":"file.txt"}).as_object().unwrap().clone();
    let normalized = oino_tools::legacy::normalize_call("read", &old, &config.cwd).unwrap();
    assert_eq!(old, json!({"path":"file.txt"}).as_object().unwrap().clone());
    assert_eq!(
        normalized.args["file_path"],
        path.to_string_lossy().as_ref()
    );
    let old = json!({"pattern":"*.txt","path":".","limit":1})
        .as_object()
        .unwrap()
        .clone();
    let normalized = oino_tools::legacy::normalize_call("find", &old, &config.cwd).unwrap();
    assert_eq!(normalized.name, "glob");
    assert_eq!(normalized.result_limit, Some(1));
    assert_eq!(normalized.args["patterns"], json!(["*.txt"]));
}

#[tokio::test]
async fn patch_model_gate() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path());
    for model in [
        "gpt-5",
        "provider/GPT-4.1",
        "codex-mini",
        "o3",
        "openai/o4-mini",
    ] {
        assert!(oino_tools::apply_patch::model_supports_apply_patch(model));
    }
    for model in ["glm-flash", "claude", "other", "foo"] {
        assert!(!oino_tools::apply_patch::model_supports_apply_patch(model));
    }
    config.context = Arc::new(Model("glm-flash"));
    assert!(tool_by_name("apply_patch", config.clone()).is_none());
    assert!(
        !oino_tools::builtin_tool_infos(&config)
            .iter()
            .any(|info| info.name.as_str() == "apply_patch")
    );
    config.context = Arc::new(Model("gpt-5"));
    run(
        &config,
        "apply_patch",
        json!({"input":"*** Begin Patch\n*** Add File: a\n+one\n+two\n*** End Patch"}),
    )
    .await
    .unwrap();
    run(&config,"apply_patch",json!({"input":"*** Begin Patch\n*** Update File: a\n*** Move to: b\n@@\n one\n-two\n+three\n*** End of File\n*** End Patch"})).await.unwrap();
    assert!(!root.path().join("a").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("b")).unwrap(),
        "one\nthree\n"
    );
    assert!(
        run(
            &config,
            "apply_patch",
            json!({"input":"*** Begin Patch\n*** Update File: b\n@@\n-absent\n+x\n*** End Patch"})
        )
        .await
        .is_err()
    );
    assert!(
        run(
            &config,
            "apply_patch",
            json!({"input":"*** Begin Patch\n*** Add File: b\n+x\n*** End Patch"})
        )
        .await
        .is_err()
    );
    assert!(
        run(
            &config,
            "apply_patch",
            json!({"input":"*** Begin Patch\n*** Update File: b\n@@\n???\n*** End Patch"})
        )
        .await
        .is_err()
    );
    run(
        &config,
        "apply_patch",
        json!({"input":"*** Begin Patch\n*** Delete File: b\n*** End Patch"}),
    )
    .await
    .unwrap();
    assert!(!root.path().join("b").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_atomic_replace() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let path = root.path().join("space 名.txt");
    std::fs::write(&path, b"\xef\xbb\xbfstart\r\n").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        drop(held);
    });
    run(
        &config,
        "write",
        json!({"file_path":path,"content":"replacement"}),
    )
    .await
    .unwrap();
    release.join().unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"\xef\xbb\xbfreplacement\r\n"
    );
}

#[tokio::test]
async fn diagnostics_only_new_and_pdf_content() {
    struct Hook;
    impl oino_tools::write::DiagnosticsHook for Hook {
        fn diagnostics(&self, _config: &ToolConfig, path: &str) -> Vec<String> {
            let mut diagnostics = vec!["existing diagnostic".to_owned()];
            if std::fs::read_to_string(path).is_ok_and(|text| text.contains("broken")) {
                diagnostics.push("new diagnostic".to_owned());
            }
            diagnostics
        }
    }
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let path = root.path().join("diagnostics");
    oino_tools::write::set_diagnostics_hook(Some(Arc::new(Hook))).unwrap();
    let result = run(
        &config,
        "write",
        json!({"file_path":path,"content":"broken"}),
    )
    .await
    .unwrap();
    assert!(text(&result).contains("new diagnostic"));
    assert!(!text(&result).contains("existing diagnostic"));
    assert_eq!(
        result.details.unwrap()["new_diagnostics"],
        json!(["new diagnostic"])
    );
    oino_tools::write::set_diagnostics_hook(None).unwrap();
    // A complete small PDF, including correct xref offsets. No external generator required.
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        { let stream = "BT /F1 12 Tf 72 720 Td (Oino PDF content) Tj ET\n"; format!("<< /Length {} >>\nstream\n{stream}endstream",stream.len()) },
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf = format!("{pdf}{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let start = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf = format!("{pdf}{offset:010} 00000 n \n");
    }
    pdf = format!("{pdf}trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n");
    let path = root.path().join("small.pdf");
    std::fs::write(&path, pdf).unwrap();
    let result = run(&config, "read", json!({"file_path":path}))
        .await
        .unwrap();
    assert!(text(&result).contains("Oino PDF content"));
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_metadata_and_failed_edits_do_not_touch_destination() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let path = root.path().join("mode");
    std::fs::write(&path, "original\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(
        run(
            &config,
            "edit",
            json!({"file_path":path,"edits":[{"oldText":"absent","newText":"x"}]})
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    run(
        &config,
        "write",
        json!({"file_path":path,"content":"replacement"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    run(
        &config,
        "write",
        json!({"file_path":link,"content":"through link"}),
    )
    .await
    .unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "through link\n");
}
