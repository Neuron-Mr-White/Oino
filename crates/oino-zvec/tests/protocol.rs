use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn index_and_query_fixture() {
    let project = tempfile::tempdir().unwrap();
    let agent = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("auth.rs"),
        "/// Authenticate a user with a password.\nfn authenticate(password: &str) {}\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_oino-zvec"))
        .env("OINO_AGENT_DIR", agent.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut request = |value: serde_json::Value| {
        writeln!(input, "{value}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], true, "{response}");
        response
    };
    request(serde_json::json!({"op":"build_index", "root":project.path()}));
    let response = request(
        serde_json::json!({"op":"query", "root":project.path(), "text":"authenticate password", "limit":10}),
    );
    assert!(response.to_string().contains("auth.rs"), "{response}");
    request(serde_json::json!({"op":"shutdown"}));
    assert!(child.wait().unwrap().success());
}
