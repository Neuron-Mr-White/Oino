//! The crate ships its own copy of `wit/oino-extension` so the published package builds
//! from crates.io. In the monorepo the canonical tree is `<repo>/wit/oino-extension`; this
//! test fails if the two drift. Outside the monorepo (packaged crate) it is a no-op.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).expect("read wit dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).expect("prefix").to_path_buf());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn bundled_wit_matches_repo_wit() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bundled = manifest.join("wit/oino-extension");
    let canonical = manifest.join("../../../wit/oino-extension");
    if !canonical.is_dir() {
        return; // packaged crate: no monorepo around it
    }
    let a = files(&canonical);
    let b = files(&bundled);
    assert_eq!(
        a, b,
        "extensions/sdk/oino-extension/wit/oino-extension drifted from wit/oino-extension \
         (file list); re-copy: rm -r extensions/sdk/oino-extension/wit/oino-extension && \
         cp -r wit/oino-extension extensions/sdk/oino-extension/wit/"
    );
    for rel in &a {
        assert_eq!(
            fs::read(canonical.join(rel)).expect("read canonical"),
            fs::read(bundled.join(rel)).expect("read bundled"),
            "{} drifted between wit/oino-extension and \
             extensions/sdk/oino-extension/wit/oino-extension",
            rel.display()
        );
    }
}
