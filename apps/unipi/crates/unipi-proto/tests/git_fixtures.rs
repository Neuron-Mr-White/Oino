//! UNI-217 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/git.test.ts`).
use serde_json::Value;
use unipi_proto::{
    Control, GitBranchesData, GitDiffData, GitLogData, GitRepoData, GitShortstatData, GitShowData,
    GitStatusData, GitSyncData, GitTurnFilesData,
};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<(), String> {
    serde_json::from_value::<T>(value)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn shared_git_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../protocol/fixtures/git.json")).unwrap();
    assert!(fixtures.len() >= 25);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "GitRepoData" => parse::<GitRepoData>(fixture.value.clone()),
            "GitStatusData" => parse::<GitStatusData>(fixture.value.clone()),
            "GitDiffData" => parse::<GitDiffData>(fixture.value.clone()),
            "GitLogData" => parse::<GitLogData>(fixture.value.clone()),
            "GitShowData" => parse::<GitShowData>(fixture.value.clone()),
            "GitBranchesData" => parse::<GitBranchesData>(fixture.value.clone()),
            "GitSyncData" => parse::<GitSyncData>(fixture.value.clone()),
            "GitShortstatData" => parse::<GitShortstatData>(fixture.value.clone()),
            "GitTurnFilesData" => parse::<GitTurnFilesData>(fixture.value.clone()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(
            result.is_ok(),
            fixture.ok,
            "fixture `{}`: {result:?}",
            fixture.name
        );
    }
}
