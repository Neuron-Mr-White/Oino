//! Contract test: the shared JSON fixtures in `protocol/fixtures/`
//! must be accepted/rejected identically by `serde` here and by zod
//! in `packages/protocol`.

use serde::Deserialize;
use serde_json::Value;
use unipi_proto::Control;

#[derive(Debug, Deserialize)]
struct ValidFixture {
    name: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
struct InvalidFixture {
    name: String,
    value: Value,
    reason: String,
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("apps")
        .join("unipi")
        .join("protocol")
        .join("fixtures")
}

fn load_valid() -> Vec<ValidFixture> {
    let path = fixtures_dir().join("control.valid.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("control.valid.json must be well-formed JSON")
}

fn load_invalid() -> Vec<InvalidFixture> {
    let path = fixtures_dir().join("control.invalid.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("control.invalid.json must be well-formed JSON")
}

#[test]
fn has_at_least_12_valid_fixtures() {
    assert!(load_valid().len() >= 12);
}

#[test]
fn has_at_least_15_invalid_fixtures() {
    assert!(load_invalid().len() >= 15);
}

#[test]
fn accepts_every_valid_fixture() {
    for fixture in load_valid() {
        let result: Result<(), _> = serde_json::from_value::<Control>(fixture.value.clone())
            .map_err(|error| error.to_string())
            .and_then(|control| control.validate().map_err(|error| error.to_string()));
        assert!(
            result.is_ok(),
            "fixture `{}` should be valid but failed: {:?}\nvalue: {}",
            fixture.name,
            result.err(),
            fixture.value
        );
    }
}

#[test]
fn rejects_every_invalid_fixture() {
    for fixture in load_invalid() {
        let result: Result<(), _> = serde_json::from_value::<Control>(fixture.value.clone())
            .map_err(|error| error.to_string())
            .and_then(|control| control.validate().map_err(|error| error.to_string()));
        assert!(
            result.is_err(),
            "fixture `{}` ({}) should be invalid but parsed as: {:?}\nvalue: {}",
            fixture.name,
            fixture.reason,
            result.ok(),
            fixture.value
        );
    }
}
