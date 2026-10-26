//! Generates `bundled.rs`: one `BundledExtension` per `extensions/dist/<id>/` (zstd-compressed
//! component, raw resources). Without a dist directory the table is empty (cargo warning).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
}

fn selected(id: &str, kind: &str) -> bool {
    let feature =
        |name: &str| std::env::var_os(format!("CARGO_FEATURE_{}", name.to_uppercase())).is_some();
    if id.starts_with("oino.fixture.") {
        feature("fixtures")
    } else if kind == "provider" {
        feature("providers")
    } else if id.starts_with("unipi.") {
        feature("unipi")
    } else {
        true
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dist = std::env::var_os("OINO_BUNDLE_DIST")
        .map_or_else(|| manifest_dir.join("../../extensions/dist"), PathBuf::from);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    println!("cargo:rerun-if-env-changed=OINO_BUNDLE_DIST");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut code = String::from("&[\n");
    let mut count = 0;
    let mut ids: Vec<PathBuf> = std::fs::read_dir(&dist)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("extension.json").is_file())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    for pkg in ids {
        println!("cargo:rerun-if-changed={}", pkg.display());
        let Ok(manifest_text) = std::fs::read_to_string(pkg.join("extension.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_text) else {
            continue;
        };
        let id = manifest["id"].as_str().unwrap_or_default().to_owned();
        let version = manifest["version"].as_str().unwrap_or_default().to_owned();
        let kind = manifest["kind"].as_str().unwrap_or("extension");
        if id.is_empty() || !selected(&id, kind) {
            continue;
        }
        let Ok(component) = std::fs::read(pkg.join("component.wasm")) else {
            continue;
        };
        let hash = sha2::Sha256::digest(&component);
        let mut hash8 = String::new();
        for b in &hash[..4] {
            let _ = write!(hash8, "{b:02x}");
        }
        let compressed = zstd::encode_all(component.as_slice(), 19).unwrap_or_default();
        let blob = out_dir.join(format!("{id}.wasm.zst"));
        if std::fs::write(&blob, compressed).is_err() {
            continue;
        }
        let mut resources = String::new();
        let mut rel_files = Vec::new();
        files(&pkg, &pkg, &mut rel_files);
        for rel in rel_files {
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if rel_s == "extension.json" || rel_s == "component.wasm" {
                continue;
            }
            let _ = writeln!(
                resources,
                "            ({rel_s:?}, include_bytes!({:?})),",
                pkg.join(&rel).display().to_string()
            );
        }
        let _ = write!(
            code,
            "    BundledExtension {{\n        id: {id:?},\n        version: {version:?},\n        manifest: include_str!({:?}),\n        component_zst: include_bytes!({:?}),\n        resources: &[\n{resources}        ],\n        hash8: {hash8:?},\n    }},\n",
            pkg.join("extension.json").display().to_string(),
            blob.display().to_string(),
        );
        count += 1;
    }
    code.push(']');
    if count == 0 {
        println!(
            "cargo:warning=oino-bundle: no components in {} (run `cargo xtask wasm`); bundling nothing",
            dist.display()
        );
    }
    let _ = std::fs::write(out_dir.join("bundled.rs"), code);
}

use sha2::Digest as _;
