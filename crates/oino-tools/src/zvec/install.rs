use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

static DOWNLOADED: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);

pub(super) fn progress() -> (u64, u64) {
    (
        DOWNLOADED.load(Ordering::Relaxed),
        TOTAL.load(Ordering::Relaxed),
    )
}

pub(super) async fn install(
    agent_dir: &Path,
    consent: bool,
    cancel: CancellationToken,
) -> Result<(), String> {
    if !consent {
        return Err("Consent is required to install semantic search".into());
    }
    let (version, asset) =
        super::client::asset().ok_or("Semantic search is not available for this build")?;
    install_asset(agent_dir, &version, &asset, cancel).await
}

async fn install_asset(
    agent_dir: &Path,
    version: &str,
    asset: &serde_json::Value,
    cancel: CancellationToken,
) -> Result<(), String> {
    let url = asset["url"].as_str().ok_or("Invalid pinned URL")?;
    let size = asset["size"].as_u64().ok_or("Invalid pinned size")?;
    let expected = asset["sha256"].as_str().ok_or("Invalid pinned checksum")?;
    let tools = agent_dir.join("tools/zvec");
    std::fs::create_dir_all(&tools).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(&tools).map_err(|e| e.to_string())?;
    let download = stage.path().join("download.tmp");
    let mut file = tokio::fs::File::create(&download)
        .await
        .map_err(|e| e.to_string())?;
    DOWNLOADED.store(0, Ordering::Relaxed);
    TOTAL.store(size, Ordering::Relaxed);
    let mut hash = Sha256::new();
    let mut count = 0u64;
    if let Some(path) = url.strip_prefix("file://") {
        use tokio::io::AsyncReadExt;
        let mut source = tokio::fs::File::open(path)
            .await
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 65536];
        loop {
            let n = tokio::select! { () = cancel.cancelled() => return Err("Installation cancelled".into()), n = source.read(&mut buffer) => n.map_err(|e| e.to_string())? };
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > size {
                return Err("Semantic search download size or SHA-256 mismatch".into());
            }
            hash.update(&buffer[..n]);
            file.write_all(&buffer[..n])
                .await
                .map_err(|e| e.to_string())?;
            DOWNLOADED.store(count, Ordering::Relaxed);
        }
    } else {
        if !url.starts_with("https://") {
            return Err("Invalid pinned URL".into());
        }
        let mut response = tokio::select! { () = cancel.cancelled() => return Err("Installation cancelled".into()), response = reqwest::get(url) => response.map_err(|e| e.to_string())? }.error_for_status().map_err(|e| e.to_string())?;
        loop {
            let chunk = tokio::select! { () = cancel.cancelled() => return Err("Installation cancelled".into()), chunk = response.chunk() => chunk.map_err(|e| e.to_string())? };
            let Some(chunk) = chunk else { break };
            count += chunk.len() as u64;
            if count > size {
                return Err("Semantic search download size or SHA-256 mismatch".into());
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            DOWNLOADED.store(count, Ordering::Relaxed);
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if count != size || format!("{:x}", hash.finalize()) != expected {
        return Err("Semantic search download size or SHA-256 mismatch".into());
    }
    let unpack = stage.path().join("unpack");
    std::fs::create_dir(&unpack).map_err(|e| e.to_string())?;
    let archive =
        flate2::read::GzDecoder::new(std::fs::File::open(&download).map_err(|e| e.to_string())?);
    let mut archive = tar::Archive::new(archive);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if path.components().count() != 1 || !entry.header().entry_type().is_file() {
            return Err("Invalid semantic search archive".into());
        }
        entry.unpack(unpack.join(path)).map_err(|e| e.to_string())?;
    }
    let exe = if cfg!(windows) {
        "oino-zvec.exe"
    } else {
        "oino-zvec"
    };
    if !unpack.join(exe).is_file() {
        return Err("Helper missing from archive".into());
    }
    if cancel.is_cancelled() {
        return Err("Installation cancelled".into());
    }
    std::fs::rename(unpack, tools.join(version)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn mismatch_leaves_nothing_installed() {
        let agent = tempfile::tempdir().unwrap();
        let source = agent.path().join("source");
        std::fs::write(&source, b"bad").unwrap();
        let asset = serde_json::json!({"url":format!("file://{}", source.display()), "size":3, "sha256":"0".repeat(64)});
        assert!(
            super::install_asset(
                agent.path(),
                "v1",
                &asset,
                tokio_util::sync::CancellationToken::new()
            )
            .await
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(!agent.path().join("tools/zvec/v1").exists());
        assert_eq!(
            std::fs::read_dir(agent.path().join("tools/zvec"))
                .unwrap()
                .count(),
            0
        );
    }
}
