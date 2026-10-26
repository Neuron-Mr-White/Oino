//! Golden tests of the public helpers (truncation, sizes, path resolution) against pi 1.4.0.
//! Fixture: `tests/fixtures/units.json` (`node units.mjs` in
//! `scripts/pi-oracle/core-runtime/tools`).

#![allow(clippy::unwrap_used, clippy::panic, reason = "test harness")]

use oino_tools::path_utils::{expand_path, resolve_to_cwd};
use oino_tools::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, GREP_MAX_LINE_LENGTH, TruncationOptions, format_size,
    truncate_head, truncate_line, truncate_tail,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/units.json")).unwrap()
}

#[test]
fn format_size_matches_pi() {
    for case in fixture()["formatSize"].as_array().unwrap() {
        assert_eq!(
            format_size(case["bytes"].as_u64().unwrap()),
            case["text"],
            "{case}"
        );
    }
}

#[test]
fn truncation_matches_pi() {
    for case in fixture()["truncation"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let options = TruncationOptions {
            max_lines: case["options"]["maxLines"]
                .as_u64()
                .unwrap_or(DEFAULT_MAX_LINES),
            max_bytes: case["options"]["maxBytes"]
                .as_u64()
                .unwrap_or(DEFAULT_MAX_BYTES),
        };
        assert_eq!(
            json!(truncate_head(text, options)),
            case["head"],
            "head {case}"
        );
        assert_eq!(
            json!(truncate_tail(text, options)),
            case["tail"],
            "tail {case}"
        );
    }
}

#[test]
fn line_truncation_matches_pi() {
    for case in fixture()["lines"].as_array().unwrap() {
        let result = truncate_line(case["line"].as_str().unwrap(), GREP_MAX_LINE_LENGTH);
        assert_eq!(
            json!({"text": result.text, "wasTruncated": result.was_truncated}),
            case["result"]
        );
    }
}

#[test]
fn path_resolution_matches_pi() {
    let home = std::env::var("HOME").unwrap();
    let normalize = |path: String| match path.strip_prefix(&home) {
        Some(rest) => format!("$HOME{rest}"),
        None => path,
    };
    for case in fixture()["resolve"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let cwd = case["cwd"].as_str().unwrap();
        assert_eq!(
            normalize(resolve_to_cwd(input, cwd).unwrap()),
            case["resolved"],
            "{case}"
        );
        assert_eq!(
            normalize(expand_path(input).unwrap()),
            case["expanded"],
            "{case}"
        );
    }
}
