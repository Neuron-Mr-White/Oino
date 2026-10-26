//! Golden suite `session-context` (quality §4.4): every fixture under `tests/fixtures/pi/`
//! is opened with [`SessionManager`] and compared with pi 1.4.0's output recorded by
//! `scripts/pi-oracle/core-types-session/context.mjs` (`<name>.expected.json`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use oino_ai::json::json_semantic_eq;
use oino_session::{FileLine, SessionManager};
use serde_json::Value;

fn fixture_files(dir: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(common::fixtures().join("pi").join(dir))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
}

fn check_snapshot(file: &Path) -> Result<(), String> {
    let expected_path = file.with_extension("expected.json");
    let mut expected: Value =
        serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();
    let temp = common::tempdir();
    let copy = temp.path().join(file.file_name().unwrap());
    fs::copy(file, &copy).unwrap();
    let mut manager = SessionManager::open(&copy, temp.path(), None).map_err(|e| e.to_string())?;
    let mut actual = common::snapshot(&mut manager);
    common::normalize_ids(&mut expected);
    common::normalize_ids(&mut actual);
    match common::first_diff(&actual, &expected, "") {
        None => Ok(()),
        Some(diff) => Err(format!("{}: {diff}", file.display())),
    }
}

fn run_suite(dir: &str, minimum: usize) {
    let files = fixture_files(dir);
    assert!(
        files.len() >= minimum,
        "{dir}: only {} fixtures",
        files.len()
    );
    let failures: Vec<String> = files
        .iter()
        .filter_map(|f| check_snapshot(f).err())
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn generated_sessions_match_pi() {
    run_suite("generated", 30);
}

#[test]
fn real_sessions_match_pi() {
    run_suite("real", 8);
}

#[test]
fn legacy_pi_versions_match_pi() {
    run_suite("legacy-pi", 2);
}

#[test]
fn doc_examples_match_pi() {
    run_suite("docs", 1);
}

/// Acceptance 1: every v3 line re-serialises to an equal JSON value.
#[test]
fn v3_lines_round_trip() {
    let mut checked = 0;
    for dir in ["generated", "real", "docs"] {
        for file in fixture_files(dir) {
            for (number, line) in fs::read_to_string(&file).unwrap().lines().enumerate() {
                let original: Value = serde_json::from_str(line).unwrap();
                let parsed = FileLine::parse(line)
                    .unwrap_or_else(|e| panic!("{}:{}: {e}", file.display(), number + 1));
                let written: Value = serde_json::from_str(&parsed.to_line().unwrap()).unwrap();
                assert!(
                    json_semantic_eq(&original, &written),
                    "{}:{}: {}",
                    file.display(),
                    number + 1,
                    common::first_diff(&written, &original, "").unwrap_or_default()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 400, "only {checked} lines");
}
