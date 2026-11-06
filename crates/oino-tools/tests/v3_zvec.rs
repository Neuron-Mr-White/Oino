//! Decision I acceptance: default builds stay exact-only and never download a model.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "acceptance assertions"
)]
use oino_agent::{AgentTool, ToolInvocation, UpdateSink};
use oino_tools::{GrepTool, ToolConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn run(
    tool: &GrepTool,
    args: Value,
) -> Result<oino_agent::AgentToolResult, oino_agent::ToolError> {
    tool.execute(
        ToolInvocation {
            tool_call_id: "zvec-test".into(),
            args: args.as_object().unwrap().clone(),
            parent_tool_call_id: None,
        },
        UpdateSink::discard(),
        CancellationToken::new(),
    )
    .await
}
fn config(root: &std::path::Path) -> ToolConfig {
    ToolConfig::new(root.to_string_lossy(), root.join("agent").to_string_lossy())
}

#[tokio::test]
async fn exact_without_index() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("code.rs"), "fn searchable() {}\n").unwrap();
    let mut config = config(root.path());
    let off = GrepTool::new(config.clone());
    assert!(
        off.meta().parameters["properties"]
            .get("search_mode")
            .is_none()
    );
    config.search.enabled = true;
    let on = GrepTool::new(config.clone());
    let catalog = oino_tools::builtin_tool_infos(&config);
    assert_eq!(
        catalog
            .iter()
            .find(|info| info.name == oino_tools::BuiltinToolName::Grep)
            .unwrap()
            .parameters,
        on.meta().parameters
    );
    assert_eq!(
        on.meta().parameters["properties"]["search_mode"]["enum"],
        json!(["exact", "semantic", "hybrid"])
    );
    config.search.enabled = false;
    assert!(
        off.meta().parameters["properties"]
            .get("search_mode")
            .is_none()
    );
    assert!(
        on.meta().parameters["properties"]
            .get("search_mode")
            .is_some()
    );
    for tool in [&off, &on] {
        assert!(
            run(tool, json!({"pattern":"searchable","search_mode":"exact"}))
                .await
                .is_ok()
        );
        assert!(run(tool, json!({"pattern":"searchable"})).await.is_ok());
    }
    assert!(
        run(&off, json!({"pattern":"search","search_mode":"semantic"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    assert!(!root.path().join(".zvec-grep").exists());
}

#[tokio::test]
async fn consent_and_missing_index() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path());
    config.search.enabled = true;
    let tool = GrepTool::new(config);
    for mode in ["semantic", "hybrid"] {
        let error = run(
            &tool,
            json!({"pattern":"authentication", "search_mode":mode, "consent":true, "build":true}),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Settings › Search › 'Build index'")
        );
        assert!(!root.path().join(".zvec-grep").exists());
    }
    assert!(
        oino_tools::zvec::build_index(root.path(), root.path(), false)
            .await
            .unwrap_err()
            .contains("consent")
    );
    assert!(!root.path().join(".zvec-grep").exists());
}

#[tokio::test]
async fn project_isolation_and_idle_unload() {
    let parent = tempfile::tempdir().unwrap();
    let child = parent.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::create_dir(parent.path().join(".zvec-grep")).unwrap();
    std::fs::write(parent.path().join(".zvec-grep/manifest.json"), "{}").unwrap();
    let mut config = config(&child);
    config.search.enabled = true;
    let tool = GrepTool::new(config);
    assert!(
        run(&tool, json!({"pattern":"query", "search_mode":"semantic"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("Build index")
    );
    assert!(!child.join(".zvec-grep").exists());
    assert_eq!(oino_tools::zvec::IDLE_TIMEOUT.as_secs(), 300);
    #[cfg(feature = "zvec")]
    {
        std::fs::write(child.join("auth.rs"), "/// Authenticate the user using a password.\nfn authenticate(password: &str) -> bool { password == \"secret\" }\n").unwrap();
        oino_tools::zvec::build_index(&child, parent.path(), true)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(child.join(".zvec-grep/.gitignore")).unwrap(),
            "*\n"
        );
        // The tool's model cache belongs to its agent directory.
        let mut config = ToolConfig::new(child.to_string_lossy(), parent.path().to_string_lossy());
        config.search.enabled = true;
        let tool = GrepTool::new(config);
        run(
            &tool,
            json!({"pattern":"authenticate password", "search_mode":"semantic"}),
        )
        .await
        .unwrap();
        run(&tool, json!({"pattern":"authenticate password", "search_mode":"hybrid", "output_mode":"content", "line_numbers":true})).await.unwrap();
        assert!(
            run(
                &tool,
                json!({"pattern":"query", "search_mode":"semantic", "path":".."})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("inside the session project")
        );
        let mut uncached = ToolConfig::new(
            child.to_string_lossy(),
            child.join("uncached").to_string_lossy(),
        );
        uncached.search.enabled = true;
        assert!(
            run(
                &GrepTool::new(uncached),
                json!({"pattern":"query", "search_mode":"semantic"})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("Build index")
        );
        assert!(!child.join("uncached").exists());
        assert!(oino_tools::zvec::loaded_models().await > 0);
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(299)).await;
        tokio::task::yield_now().await;
        assert!(oino_tools::zvec::loaded_models().await > 0);
        tokio::time::advance(std::time::Duration::from_millis(1010)).await;
        tokio::task::yield_now().await;
        assert_eq!(oino_tools::zvec::loaded_models().await, 0);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_msvc_cpu() {
    assert_eq!(std::env::consts::OS, "windows");
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("test.rs"), "fn authenticate() {}\n").unwrap();
    assert!(
        run(
            &GrepTool::new(config(root.path())),
            json!({"pattern":"authenticate"})
        )
        .await
        .is_ok()
    );
    #[cfg(feature = "zvec")]
    {
        oino_tools::zvec::build_index(root.path(), root.path(), true)
            .await
            .unwrap();
        let mut config = config(root.path());
        config.agent_dir = root.path().to_string_lossy().into_owned();
        config.search.enabled = true;
        assert!(
            run(
                &GrepTool::new(config),
                json!({"pattern":"authenticate", "search_mode":"hybrid"})
            )
            .await
            .is_ok()
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_index_cannot_escape_project() {
    let project = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(external.path(), project.path().join(".zvec-grep")).unwrap();
    assert!(
        oino_tools::zvec::build_index(project.path(), project.path(), true)
            .await
            .unwrap_err()
            .contains("symlink")
    );
    assert!(!external.path().join(".gitignore").exists());
}
