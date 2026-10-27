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

/// unipi load order (`unipi_kit::suite::LOAD_ORDER`, extensions workspace); keep in sync.
const UNIPI_LOAD_ORDER: &[&str] = &[
    "core",
    "workflow",
    "long-horizon",
    "memory",
    "dream",
    "utility",
    "skill-registry",
    "info-screen",
    "subagents",
    "background-tasks",
    "btw",
    "web-api",
    "ask-user",
    "mcp",
    "notify",
    "cron",
    "kanboard",
    "command-enchantment",
    "compactor",
    "footer",
    "updater",
    "input-shortcuts",
    "fusion",
    "watchdog",
    "app-bridge",
];

/// Bundle position: providers, then unipi in load order, then everything else (by id).
fn load_rank(id: &str, kind: &str) -> (u8, usize) {
    if kind == "provider" {
        return (0, 0);
    }
    match id.strip_prefix("unipi.") {
        Some(name) => (
            1,
            UNIPI_LOAD_ORDER
                .iter()
                .position(|n| *n == name)
                .unwrap_or(UNIPI_LOAD_ORDER.len()),
        ),
        None => (2, 0),
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
    let mut packages: Vec<(PathBuf, serde_json::Value)> = ids
        .into_iter()
        .filter_map(|pkg| {
            println!("cargo:rerun-if-changed={}", pkg.display());
            let text = std::fs::read_to_string(pkg.join("extension.json")).ok()?;
            let manifest = serde_json::from_str::<serde_json::Value>(&text).ok()?;
            Some((pkg, manifest))
        })
        .collect();
    // Stable sort keeps the directory order inside a rank.
    packages.sort_by_key(|(_, m)| {
        load_rank(
            m["id"].as_str().unwrap_or_default(),
            m["kind"].as_str().unwrap_or("extension"),
        )
    });
    for (pkg, manifest) in packages {
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
    write_docs(&manifest_dir.join("../.."), &out_dir);
}

/// `docs.rs`: `README.md` and `docs/**/*.md` except `docs/dev/` (developer specs).
fn write_docs(repo: &Path, out_dir: &Path) {
    let mut rel_files = vec![PathBuf::from("README.md")];
    let mut docs = Vec::new();
    files(&repo.join("docs"), repo, &mut docs);
    rel_files.extend(
        docs.into_iter()
            .filter(|p| !p.starts_with("docs/dev") && p.extension().is_some_and(|e| e == "md")),
    );
    println!(
        "cargo:rerun-if-changed={}",
        repo.join("README.md").display()
    );
    println!("cargo:rerun-if-changed={}", repo.join("docs").display());
    let mut hasher = sha2::Sha256::new();
    let mut entries = String::new();
    for rel in &rel_files {
        let path = repo.join(rel);
        println!("cargo:rerun-if-changed={}", path.display());
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        hasher.update(rel_s.as_bytes());
        hasher.update(&bytes);
        let _ = writeln!(
            entries,
            "        ({rel_s:?}, include_bytes!({:?})),",
            path.display().to_string()
        );
    }
    let hash = hasher.finalize();
    let mut hash8 = String::new();
    for b in &hash[..4] {
        let _ = write!(hash8, "{b:02x}");
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let code = format!(
        "BundledDocs {{\n    version: {version:?},\n    hash8: {hash8:?},\n    files: &[\n{entries}    ],\n}}"
    );
    let _ = std::fs::write(out_dir.join("docs.rs"), code);
}

use sha2::Digest as _;
