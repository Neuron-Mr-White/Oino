use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[cfg(not(feature = "zvec"))]
use std::process::Stdio;
#[cfg(not(feature = "zvec"))]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MANIFEST: &str = include_str!(concat!(env!("OUT_DIR"), "/zvec-manifest.json"));

pub(super) fn asset() -> Option<(String, Value)> {
    asset_from(MANIFEST)
}

fn asset_from(source: &str) -> Option<(String, Value)> {
    let manifest: Value = serde_json::from_str(source).ok()?;
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

#[cfg(not(feature = "zvec"))]
pub(super) fn helper(agent_dir: &Path) -> Option<PathBuf> {
    helper_for(agent_dir, asset())
}

fn helper_for(agent_dir: &Path, asset: Option<(String, Value)>) -> Option<PathBuf> {
    let (version, _) = asset?;
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
struct Process {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

#[cfg(not(feature = "zvec"))]
type Pool = std::collections::HashMap<(PathBuf, PathBuf), Process>;
#[cfg(not(feature = "zvec"))]
static POOL: std::sync::OnceLock<tokio::sync::Mutex<Pool>> = std::sync::OnceLock::new();

#[cfg(not(feature = "zvec"))]
fn spawn(agent_dir: &Path, path: &Path) -> Result<Process, String> {
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
    let mut child = tokio::process::Command::new(path)
        .env(library_var, library_path)
        .env("OINO_AGENT_DIR", agent_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let input = child.stdin.take().ok_or("helper stdin unavailable")?;
    let output = BufReader::new(child.stdout.take().ok_or("helper stdout unavailable")?).lines();
    Ok(Process {
        child,
        input,
        output,
    })
}

#[cfg(not(feature = "zvec"))]
pub(super) async fn call(agent_dir: &Path, request: Value) -> Result<Value, String> {
    let path = helper(agent_dir).ok_or(super::INSTALL)?;
    let root = request
        .get("root")
        .and_then(Value::as_str)
        .ok_or("helper root missing")?;
    let key = (agent_dir.to_path_buf(), PathBuf::from(root));
    let pool = POOL.get_or_init(|| tokio::sync::Mutex::new(Pool::new()));
    let mut pool = pool.lock().await;
    if let Some(process) = pool.get_mut(&key)
        && process
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
    {
        pool.remove(&key);
    }
    if !pool.contains_key(&key) {
        pool.insert(key.clone(), spawn(agent_dir, &path)?);
    }
    let process = pool.get_mut(&key).ok_or("helper unavailable")?;
    let response = async {
        process
            .input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        process
            .output
            .next_line()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("helper exited without response".to_string())
    }
    .await;
    let line = match response {
        Ok(line) => line,
        Err(error) => {
            pool.remove(&key);
            return Err(error);
        }
    };
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
    capabilities_for(agent_dir, asset())
}

fn capabilities_for(agent_dir: &Path, asset: Option<(String, Value)>) -> Value {
    let installed = cfg!(feature = "zvec") || helper_for(agent_dir, asset.clone()).is_some();
    json!({"semanticSearch": installed, "installed": installed, "available": asset.is_some(), "size": asset.as_ref().and_then(|(_, a)| a.get("size")).and_then(Value::as_u64), "version": asset.map(|(v, _)| v)})
}

#[cfg(all(test, not(feature = "zvec")))]
mod tests {
    #[test]
    fn pinned_capability_tracks_installation() {
        let dir = tempfile::tempdir().unwrap();
        let target = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
            ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("macos", "aarch64") => "aarch64-apple-darwin",
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            _ => return,
        };
        let manifest =
            serde_json::json!({"version":"1.2.3", "assets":{target:{"size":123}}}).to_string();
        let asset = super::asset_from(&manifest);
        assert_eq!(
            super::capabilities_for(dir.path(), asset.clone())["installed"],
            false
        );
        let install = dir.path().join("tools/zvec/1.2.3");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(
            install.join(if cfg!(windows) {
                "oino-zvec.exe"
            } else {
                "oino-zvec"
            }),
            "fixture",
        )
        .unwrap();
        assert_eq!(
            super::capabilities_for(dir.path(), asset)["installed"],
            true
        );
    }
}
