//! `cargo xtask <task>` (alias in `.cargo/config.toml`).
//!
//! | Task | Does |
//! |---|---|
//! | `wasm [--debug] [-p <crate>…]` | builds every component under `extensions/{providers,unipi,oino,fixtures}` for `wasm32-wasip2`, builds any declared `binaries` (ext-abi §5.2) for the host target and writes `extensions/dist/<id>/{extension.json, component.wasm, resources, bin/}` + `extensions/dist/checksums.json` |
//! | `checksums` | verifies `extensions/dist` against `checksums.json` |
//! | `wit` | validates `wit/oino-extension` with `wasm-tools component wit` |
//! | `bench <suite> [--n N] [--out dir]` | core benchmarks (WP-C11): runs a suite via `crates/oino-bench --release`; results land under `bench/results/` (currently: `subagents`) |
//! | `remote-build <host\|local> [opts] [-- build-host args]` | release build over SSH (or locally) via `scripts/release/build-host.sh`; artifacts in `target/release-artifacts/<target>/` (see `remote.rs`) |
//! | `ti <sync\|build>` | release build host on the `ti` Ubuntu box: `sync` the tree there, `build` the CLI (see `ti.rs`; docs/dev/v1/ti.md is historical — its GUI-test-host commands went with the GPUI desktop app on v2) |
//! | `gui <list\|suite> <name>` | the driver's regression suites: runs every scenario of `docs/dev/v1/gui/suites/<name>.json` through `scripts/gui-drive.py` (see `gui.rs`, docs/dev/v1/gui-driver.md) |

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod bench;
mod gui;
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

/// The host target triple (`rustc -vV` `host:`), so native binaries land at a
/// stable `target/<triple>/<profile>/` path regardless of local cargo config.
fn host_target() -> Result<String> {
    let out = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg("-vV")
        .output()
        .context("rustc -vV")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let host = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("host:"))
        .map(str::trim)
        .with_context(|| format!("rustc -vV has no host line: {text:?}"))?;
    Ok(host.to_owned())
}

/// The root-workspace crate whose `[[bin]]` builds `bin` (ext-abi §5.2: an
/// extension package ships native CLI binaries built by the monorepo — the
/// extensions workspace cannot build them, it targets wasm guests). The bin
/// name is also the crate name in this repo (e.g. `unipi-kanboard`).
fn owning_crate(bin: &str) -> Result<String> {
    let crates_dir = root().join("crates");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let Ok(toml) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let mut section = "";
        let mut declared = false;
        for line in toml.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                section = trimmed;
                continue;
            }
            let Some(name) = trimmed.strip_prefix("name = ") else {
                continue;
            };
            let name = name.trim_matches('"');
            if name == bin && (section == "[[bin]]" || section == "[package]") {
                declared = true;
            }
        }
        if declared && dir.join("src/main.rs").is_file() {
            return Ok(bin.to_owned());
        }
    }
    bail!("no crate under crates/ builds binary `{bin}`")
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line build: component, resources, binaries, checksums"
)]
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
        let mut pkg_checksums = serde_json::Map::new();
        pkg_checksums.insert("component.wasm".to_owned(), json!(sha256(&component)));
        // ext-abi §5.2: packages declaring `binaries` ship this platform's
        // build in `bin/` — the dist is therefore target-specific.
        let binaries: Vec<String> = c.manifest["binaries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
        if !binaries.is_empty() {
            let target = host_target()?;
            let mut crates: Vec<String> = Vec::new();
            for bin in &binaries {
                let owner = owning_crate(bin)?;
                if !crates.contains(&owner) {
                    crates.push(owner);
                }
            }
            println!(
                "building native binaries {} for {target}",
                crates.join(", ")
            );
            let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            cmd.current_dir(root()).arg("build");
            for owner in &crates {
                cmd.arg("-p").arg(owner);
            }
            if !debug {
                cmd.arg("--release");
            }
            cmd.arg("--target").arg(&target);
            let status = cmd.status()?;
            if !status.success() {
                bail!("cargo build failed for {}", crates.join(", "));
            }
            let exe = |bin: &str| {
                let is_exe = std::path::Path::new(bin)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
                if cfg!(windows) && !is_exe {
                    format!("{bin}.exe")
                } else {
                    bin.to_owned()
                }
            };
            for bin in &binaries {
                let built = root()
                    .join("target")
                    .join(&target)
                    .join(profile)
                    .join(exe(bin));
                let out_bin = out_dir.join("bin").join(exe(bin));
                let bin_dir = out_bin.parent().context("bin/ has a parent")?;
                std::fs::create_dir_all(bin_dir)?;
                std::fs::copy(&built, &out_bin)
                    .with_context(|| format!("copying {}", built.display()))?;
                pkg_checksums.insert(
                    format!("bin/{}", exe(bin)),
                    json!(sha256(&std::fs::read(&out_bin)?)),
                );
                files.push(out_bin);
            }
        }
        manifest["checksums"] = Value::Object(pkg_checksums);
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
        Some("bench") => bench::bench(&args[1..]),
        Some("remote-build") => remote::remote_build(&args[1..]),
        Some("ti") => ti::ti(&args[1..]),
        Some("gui") => gui::gui(&args[1..]),
        _ => bail!(
            "usage: cargo xtask <wasm [--debug] [-p crate]… | checksums | wit | \
             bench <subagents --n N> [--out dir] | \
             remote-build <host|local> … | ti <sync|build> … | \
             gui <list|suite> <name> …>"
        ),
    }
}
