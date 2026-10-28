//! `cargo xtask <task>` (alias in `.cargo/config.toml`).
//!
//! | Task | Does |
//! |---|---|
//! | `wasm [--debug] [-p <crate>…]` | builds every component under `extensions/{providers,unipi,fixtures}` for `wasm32-wasip2` and writes `extensions/dist/<id>/{extension.json, component.wasm, resources}` + `extensions/dist/checksums.json` |
//! | `checksums` | verifies `extensions/dist` against `checksums.json` |
//! | `wit` | validates `wit/oino-extension` with `wasm-tools component wit` |
//! | `remote-build <host\|local> [opts] [-- build-host args]` | release build over SSH (or locally) via `scripts/release/build-host.sh`; artifacts in `target/release-artifacts/<target>/` (see `remote.rs`) |
//! | `ti <sync\|build\|start\|shot\|status\|soak\|stop>` | the `ti` Ubuntu box as GUI test host: private `Xvfb :93`, scratch agent dir, screenshots, soak (see `ti.rs`, docs/dev/v1/ti.md) |

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod remote;
mod ti;

const GROUPS: &[&str] = &["providers", "unipi", "oino", "fixtures"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn sha256(bytes: &[u8]) -> String {
    let mut out = String::from("sha256:");
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// One component crate with an `extension.json`.
struct Component {
    dir: PathBuf,
    crate_name: String,
    manifest: Value,
}

fn crate_name(dir: &Path) -> Result<String> {
    let toml = std::fs::read_to_string(dir.join("Cargo.toml"))?;
    toml.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("name = ")
                .map(|v| v.trim_matches('"').to_owned())
        })
        .with_context(|| format!("{}: no package name", dir.display()))
}

fn components(filter: &[String]) -> Result<Vec<Component>> {
    let ext = root().join("extensions");
    let mut out = Vec::new();
    for group in GROUPS {
        let Ok(entries) = std::fs::read_dir(ext.join(group)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("extension.json").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            let crate_name = crate_name(&dir)?;
            if !filter.is_empty() && !filter.contains(&crate_name) {
                continue;
            }
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(dir.join("extension.json"))?)?;
            out.push(Component {
                dir,
                crate_name,
                manifest,
            });
        }
    }
    Ok(out)
}

fn copy_dir(from: &Path, to: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, files)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            files.push(target);
        }
    }
    Ok(())
}

fn tool_version(cmd: &str, arg: &str) -> String {
    Command::new(cmd)
        .arg(arg)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn wasm(args: &[String]) -> Result<()> {
    let debug = args.iter().any(|a| a == "--debug");
    let filter: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "-p")
        .map(|w| w[1].clone())
        .collect();
    let ext = root().join("extensions");
    let dist = ext.join("dist");
    let profile = if debug { "debug" } else { "release" };
    let components = components(&filter)?;
    if components.is_empty() {
        bail!(
            "no components found under extensions/{{{}}}",
            GROUPS.join(",")
        );
    }
    let mut checksums = BTreeMap::new();
    for c in &components {
        // One cargo invocation per component: feature unification (e.g. the SDK `provider`
        // feature) must not leak between components.
        println!("building {}", c.crate_name);
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.current_dir(&ext)
            .args(["build", "--target", "wasm32-wasip2", "-p", &c.crate_name]);
        if !debug {
            cmd.arg("--release");
        }
        let status = cmd.status()?;
        if !status.success() {
            bail!("cargo build failed for {}", c.crate_name);
        }
        let id = c.manifest["id"].as_str().context("manifest id")?.to_owned();
        let built = ext
            .join("target/wasm32-wasip2")
            .join(profile)
            .join(format!("{}.wasm", c.crate_name.replace('-', "_")));
        let out_dir = dist.join(&id);
        if out_dir.exists() {
            std::fs::remove_dir_all(&out_dir)?;
        }
        std::fs::create_dir_all(&out_dir)?;
        let component = std::fs::read(&built).with_context(|| built.display().to_string())?;
        std::fs::write(out_dir.join("component.wasm"), &component)?;
        let mut files = vec![out_dir.join("component.wasm")];
        let resources = &c.manifest["resources"];
        let mut resource_dirs: Vec<&str> = ["skills", "prompts", "themes"]
            .iter()
            .flat_map(|k| {
                resources[*k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
            })
            .collect();
        resource_dirs.extend(resources["docs"].as_str());
        for rel in resource_dirs {
            copy_dir(&c.dir.join(rel), &out_dir.join(rel), &mut files)?;
        }
        let mut manifest = c.manifest.clone();
        manifest["component"] = json!("component.wasm");
        manifest["checksums"] = json!({"component.wasm": sha256(&component)});
        std::fs::write(
            out_dir.join("extension.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        files.push(out_dir.join("extension.json"));
        for f in files {
            let rel = f.strip_prefix(&dist)?.to_string_lossy().replace('\\', "/");
            checksums.insert(rel, sha256(&std::fs::read(&f)?));
        }
    }
    if filter.is_empty() {
        let doc = json!({
            "toolchain": tool_version("rustc", "-V"),
            "wasmTools": tool_version("wasm-tools", "--version"),
            "profile": profile,
            "files": checksums,
        });
        std::fs::write(
            dist.join("checksums.json"),
            serde_json::to_vec_pretty(&doc)?,
        )?;
    }
    println!(
        "wrote {} component(s) to {}",
        components.len(),
        dist.display()
    );
    Ok(())
}

fn checksums() -> Result<()> {
    let dist = root().join("extensions/dist");
    let doc: Value = serde_json::from_slice(&std::fs::read(dist.join("checksums.json"))?)?;
    let mut bad = 0;
    for (rel, expected) in doc["files"].as_object().context("files")? {
        let actual =
            std::fs::read(dist.join(rel)).map_or_else(|_| "missing".into(), |b| sha256(&b));
        if Some(actual.as_str()) != expected.as_str() {
            eprintln!("mismatch: {rel}");
            bad += 1;
        }
    }
    if bad > 0 {
        bail!("{bad} file(s) differ from checksums.json");
    }
    println!("extensions/dist matches checksums.json");
    Ok(())
}

fn wit() -> Result<()> {
    let status = Command::new("wasm-tools")
        .args(["component", "wit"])
        .arg(root().join("wit/oino-extension"))
        .args(["-o", if cfg!(windows) { "NUL" } else { "/dev/null" }])
        .status()
        .context("wasm-tools not found (cargo install wasm-tools)")?;
    if !status.success() {
        bail!("wit/oino-extension does not validate");
    }
    println!("wit/oino-extension validates");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("wasm") => wasm(&args[1..]),
        Some("checksums") => checksums(),
        Some("wit") => wit(),
        Some("remote-build") => remote::remote_build(&args[1..]),
        Some("ti") => ti::ti(&args[1..]),
        _ => bail!(
            "usage: cargo xtask <wasm [--debug] [-p crate]… | checksums | wit | \
             remote-build <host|local> … | ti <sync|build|start|shot|status|soak|stop> …>"
        ),
    }
}
