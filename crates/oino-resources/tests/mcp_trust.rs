//! All project MCP compatibility paths require saved trust.
#![allow(clippy::unwrap_used, reason = "test assertions")]
use oino_resources::trust::{TrustStore, has_protected_resources, project_trust_snapshot};

#[test]
fn every_mcp_compatibility_file_requires_trust_without_reading_content() {
    for file in [
        ".oino/mcp.json",
        ".pi/mcp.json",
        ".unipi/config/mcp/mcp-config.json",
        ".mcp.json",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let agent = temp.path().join("agent");
        let path = project.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(path, "invalid JSON must not be parsed").unwrap();
        assert!(has_protected_resources(&project, None), "{file}");
        let snapshot = project_trust_snapshot(&project, &agent).unwrap();
        assert_eq!(snapshot["state"], "undecided");
        assert_eq!(snapshot["protected"], serde_json::json!([file]));
        TrustStore::new(&agent)
            .set(temp.path(), Some(true))
            .unwrap();
        assert_eq!(
            project_trust_snapshot(&project, &agent).unwrap()["state"],
            "trusted"
        );
        TrustStore::new(&agent).set(&project, Some(false)).unwrap();
        assert_eq!(
            project_trust_snapshot(&project, &agent).unwrap()["state"],
            "untrusted"
        );
    }
}
