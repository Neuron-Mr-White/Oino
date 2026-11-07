use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[cfg(not(feature = "zvec"))]
use std::process::Stdio;
#[cfg(not(feature = "zvec"))]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MANIFEST: &str = include_str!("zvec-manifest.json");

pub(super) fn asset() -> Option<(String, Value)> {
    let manifest: Value = serde_json::from_str(MANIFEST).ok()?;
    let version = manifest.get("version")?.as_str()?;
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        return None;
    }
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    Some((
        version.to_owned(),
        manifest.get("assets")?.get(target)?.clone(),
    ))
}

pub(super) fn helper(agent_dir: &Path) -> Option<PathBuf> {
    let (version, _) = asset()?;
    let path = agent_dir
        .join("tools/zvec")
        .join(version)
        .join(if cfg!(windows) {
            "oino-zvec.exe"
        } else {
            "oino-zvec"
        });
    path.is_file().then_some(path)
}

#[cfg(not(feature = "zvec"))]
pub(super) async fn call(agent_dir: &Path, request: Value) -> Result<Value, String> {
    let path = helper(agent_dir).ok_or(super::INSTALL)?;
    let library_dir = path.parent().ok_or("Invalid helper path")?;
    let library_var = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else if cfg!(windows) {
        "PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let mut paths = vec![library_dir.to_path_buf()];
    if let Some(existing) = std::env::var_os(library_var) {
        paths.extend(std::env::split_paths(&existing));
    }
    let library_path = std::env::join_paths(paths).map_err(|e| e.to_string())?;
    let mut child = tokio::process::Command::new(&path)
        .env(library_var, library_path)
        .env("OINO_AGENT_DIR", agent_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().ok_or("helper stdin unavailable")?;
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let output = child.stdout.take().ok_or("helper stdout unavailable")?;
    let mut lines = BufReader::new(output).lines();
    let line = lines
        .next_line()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("helper exited without response")?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if reply["ok"] == true {
        Ok(reply["result"].clone())
    } else {
        Err(reply["error"]
            .as_str()
            .unwrap_or("helper failed")
            .to_owned())
    }
}

pub(super) fn capabilities(agent_dir: &Path) -> Value {
    let asset = asset();
    json!({"semanticSearch": cfg!(feature = "zvec") || helper(agent_dir).is_some(), "installed": cfg!(feature = "zvec") || helper(agent_dir).is_some(), "available": asset.is_some(), "size": asset.as_ref().and_then(|(_, a)| a.get("size")).and_then(Value::as_u64), "version": asset.map(|(v, _)| v)})
}
