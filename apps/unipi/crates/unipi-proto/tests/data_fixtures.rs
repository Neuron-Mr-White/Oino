//! Contract test: the shared JSON fixtures in `protocol/fixtures/`
//! for the typed `ok.data` payloads (`PairData`) must be
//! accepted/rejected identically by `serde` here and by zod in
//! `packages/protocol`.

use serde::Deserialize;
use serde_json::Value;
use unipi_proto::PairData;

#[derive(Debug, Deserialize)]
struct ValidFixture {
    name: String,
    schema: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
struct InvalidFixture {
    name: String,
    schema: String,
    value: Value,
    reason: String,
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("protocol")
        .join("fixtures")
}

fn load_valid() -> Vec<ValidFixture> {
    let path = fixtures_dir().join("data.valid.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("data.valid.json must be well-formed JSON")
}

fn load_invalid() -> Vec<InvalidFixture> {
    let path = fixtures_dir().join("data.invalid.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("data.invalid.json must be well-formed JSON")
}

fn parses_as(schema: &str, value: &Value) -> Result<(), String> {
    match schema {
        "PairData" => serde_json::from_value::<PairData>(value.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        other => panic!("unknown schema in fixture: {other}"),
    }
}

#[test]
fn has_at_least_2_valid_fixtures_per_schema() {
    let valid = load_valid();
    let count = valid.iter().filter(|f| f.schema == "PairData").count();
    assert!(count >= 2, "expected >=2 valid fixtures for PairData");
}

#[test]
fn has_at_least_2_invalid_fixtures_per_schema() {
    let invalid = load_invalid();
    let count = invalid.iter().filter(|f| f.schema == "PairData").count();
    assert!(count >= 2, "expected >=2 invalid fixtures for PairData");
}

#[test]
fn accepts_every_valid_fixture() {
    for fixture in load_valid() {
        let result = parses_as(&fixture.schema, &fixture.value);
        assert!(
            result.is_ok(),
            "fixture `{}` ({}) should be valid but failed: {:?}\nvalue: {}",
            fixture.name,
            fixture.schema,
            result.err(),
            fixture.value
        );
    }
}

#[test]
fn rejects_every_invalid_fixture() {
    for fixture in load_invalid() {
        let result = parses_as(&fixture.schema, &fixture.value);
        assert!(
            result.is_err(),
            "fixture `{}` ({}, {}) should be invalid but parsed ok\nvalue: {}",
            fixture.name,
            fixture.schema,
            fixture.reason,
            fixture.value
        );
    }
}
