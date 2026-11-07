use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn verify(bytes: &[u8], size: u64, expected: &str) -> Result<(), String> {
    if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err("Semantic search download size or SHA-256 mismatch".into());
    }
    Ok(())
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
    let url = asset["url"].as_str().ok_or("Invalid pinned URL")?;
    if !url.starts_with("https://") {
        return Err("Invalid pinned URL".into());
    }
    let size = asset["size"].as_u64().ok_or("Invalid pinned size")?;
    let hash = asset["sha256"].as_str().ok_or("Invalid pinned checksum")?;
    let response = tokio::select! {
        () = cancel.cancelled() => return Err("Installation cancelled".into()),
        response = reqwest::get(url) => response.map_err(|e| e.to_string())?,
    }
    .error_for_status()
    .map_err(|e| e.to_string())?;
    let bytes = tokio::select! {
        () = cancel.cancelled() => return Err("Installation cancelled".into()),
        bytes = response.bytes() => bytes.map_err(|e| e.to_string())?,
    };
    verify(&bytes, size, hash)?;
    let tools = agent_dir.join("tools/zvec");
    std::fs::create_dir_all(&tools).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(&tools).map_err(|e| e.to_string())?;
    let archive = flate2::read::GzDecoder::new(bytes.as_ref());
    let mut archive = tar::Archive::new(archive);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if path.components().count() != 1 || !entry.header().entry_type().is_file() {
            return Err("Invalid semantic search archive".into());
        }
        entry
            .unpack(stage.path().join(path))
            .map_err(|e| e.to_string())?;
    }
    let exe = if cfg!(windows) {
        "oino-zvec.exe"
    } else {
        "oino-zvec"
    };
    if !stage.path().join(exe).is_file() {
        return Err("Helper missing from archive".into());
    }
    if cancel.is_cancelled() {
        return Err("Installation cancelled".into());
    }
    std::fs::rename(stage.path(), tools.join(version)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn mismatch_is_rejected() {
        assert!(super::verify(b"bad", 3, &"0".repeat(64)).is_err());
    }
}
