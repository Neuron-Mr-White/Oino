//! CloakBrowser (UNI-292): a Chromium build with C++ fingerprint patches
//! (<https://github.com/CloakHQ/CloakBrowser>), the default engine of the
//! host's dedicated browser.
//!
//! **Licence.** The binary's own licence (BINARY-LICENSE.md) forbids
//! redistributing or modifying it; running it with flags and profiles is
//! fine. So it is never bundled or vendored: the host downloads the official
//! **free** build on first use, exactly the way the MIT wrapper
//! (`cloakbrowser` on npm / PyPI, here mirrored from 0.6.2) does it:
//!
//! 1. `https://cloakbrowser.dev/chromium-v<version>/SHA256SUMS` and
//!    `SHA256SUMS.sig` (falling back to the same files on GitHub Releases,
//!    `https://github.com/CloakHQ/cloakbrowser/releases/download/chromium-v<version>/`),
//!    both from the same origin;
//! 2. the detached signature (base64 of a raw 64-byte Ed25519 signature over
//!    the exact manifest bytes) must validate against one of the pinned keys
//!    in [`SIGNING_PUBKEYS`] — non-bypassable;
//! 3. the manifest's `version=<v>` line must be the version we asked for
//!    (no forced downgrade to an older genuinely-signed release);
//! 4. the archive `cloakbrowser-<platform>.tar.gz` (`.zip` on Windows) must
//!    hash to the manifest's SHA-256 for it.
//!
//! Anything else is refused. The archive is extracted into a private
//! staging dir and renamed into `~/.cache/unipi/cloakbrowser/<version>/`
//! (idempotent; concurrent callers in this process are serialised). A build
//! the wrapper already downloaded (`~/.cloakbrowser/chromium-<version>/`, or
//! `$CLOAKBROWSER_CACHE_DIR`) is reused as is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use sha2::Digest;

/// The wrapper release these pins were read from.
pub const WRAPPER_VERSION: &str = "0.6.2";
/// Primary download origin.
pub const DOWNLOAD_BASE_URL: &str = "https://cloakbrowser.dev";
/// Mirror (GitHub Releases).
pub const GITHUB_DOWNLOAD_BASE_URL: &str = "https://github.com/CloakHQ/cloakbrowser/releases/download";

/// Ed25519 public keys (base64 of the raw 32 bytes) that sign `SHA256SUMS`.
/// From the wrapper's `BINARY_SIGNING_PUBKEYS`; several allowed for rotation.
pub const SIGNING_PUBKEYS: &[&str] = &["MKFKwIhUcKWq5xTuNA0Ovg99njcDEcEJvmWYYhApvaU="];

/// The free build per platform (the wrapper's `PLATFORM_CHROMIUM_VERSIONS`).
pub const PLATFORM_VERSIONS: &[(&str, &str)] = &[
    ("linux-x64", "146.0.7680.177.5"),
    ("linux-arm64", "146.0.7680.177.3"),
    ("darwin-arm64", "145.0.7632.109.2"),
    ("darwin-x64", "145.0.7632.109.2"),
    ("windows-x64", "146.0.7680.177.5"),
];

/// A persistent profile's fingerprint seed (decimal + newline), the same
/// file every wrapper uses, so a profile keeps one identity across sessions.
pub const SEED_FILE: &str = ".cloakbrowser-seed";
pub const SEED_MIN: u32 = 10_000;
pub const SEED_MAX: u32 = 99_999;

/// Overrides where the host keeps downloaded builds (tests).
pub const CACHE_ENV: &str = "UNIPI_CLOAKBROWSER_CACHE";
/// The wrapper's own cache override (also honoured when looking for a reusable build).
pub const WRAPPER_CACHE_ENV: &str = "CLOAKBROWSER_CACHE_DIR";

/// What people see as the browser's name.
pub const DISPLAY_NAME: &str = "CloakBrowser";

/// This machine's platform tag, when CloakBrowser ships a build for it.
pub fn platform_tag() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("darwin-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("darwin-x64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

/// The pinned free version for `tag`.
pub fn pinned_version(tag: &str) -> Option<&'static str> {
    PLATFORM_VERSIONS.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

/// A full numeric Chromium version: `146.0.7680.177` or `146.0.7680.177.5`.
pub fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (4..=5).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `cloakbrowser-<tag>.tar.gz`, `.zip` for Windows.
pub fn archive_name(tag: &str) -> String {
    let ext = if tag.starts_with("windows") { ".zip" } else { ".tar.gz" };
    format!("cloakbrowser-{tag}{ext}")
}

/// `<base>/chromium-v<version>/<file>`.
pub fn release_url(base: &str, version: &str, file: &str) -> String {
    format!("{base}/chromium-v{version}/{file}")
}

/// The executable inside an extracted build.
pub fn binary_in(dir: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        dir.join("Chromium.app").join("Contents").join("MacOS").join("Chromium")
    } else if cfg!(windows) {
        dir.join("chrome.exe")
    } else {
        dir.join("chrome")
    }
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `~/.cache/unipi/cloakbrowser` (`$XDG_CACHE_HOME`, macOS `~/Library/Caches`,
/// Windows `%LOCALAPPDATA%`), or `$UNIPI_CLOAKBROWSER_CACHE`.
pub fn cache_root() -> Option<PathBuf> {
    if let Some(dir) = env_dir(CACHE_ENV) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env_dir("LOCALAPPDATA")?
    } else if cfg!(target_os = "macos") {
        env_dir("HOME")?.join("Library").join("Caches")
    } else {
        env_dir("XDG_CACHE_HOME").unwrap_or(env_dir("HOME")?.join(".cache"))
    };
    Some(base.join("unipi").join("cloakbrowser"))
}

/// Where the wrapper keeps its builds (`~/.cloakbrowser`).
pub fn wrapper_cache_root() -> Option<PathBuf> {
    env_dir(WRAPPER_CACHE_ENV).or_else(|| Some(crate::fs::home_dir_opt()?.join(".cloakbrowser")))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

// ------------------------------------------------------------ verification

/// Verifies the detached `SHA256SUMS.sig` (base64 text) over the exact
/// manifest bytes with any of `keys` (base64 raw Ed25519 public keys).
pub fn verify_signature(manifest: &[u8], sig_file: &[u8], keys: &[&str]) -> anyhow::Result<()> {
    let text = std::str::from_utf8(sig_file).map_err(|_| anyhow::anyhow!("malformed SHA256SUMS.sig (not text)"))?.trim();
    // Strict: padding required, invalid characters and non-zero trailing bits refused.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| anyhow::anyhow!("malformed SHA256SUMS.sig (not valid base64)"))?;
    let raw: [u8; 64] = raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("malformed SHA256SUMS.sig (not a 64-byte Ed25519 signature)"))?;
    let signature = ed25519_dalek::Signature::from_bytes(&raw);
    for key in keys {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(key) else { continue };
        let Ok(bytes) = <[u8; 32]>::try_from(bytes) else { continue };
        let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&bytes) else { continue };
        if key.verify_strict(manifest, &signature).is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("SHA256SUMS signature verification failed — no pinned CloakBrowser key validated the manifest")
}

/// The manifest's `version=<v>` line.
pub fn parse_manifest_version(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|l| l.strip_prefix("version=")).map(|v| v.trim().to_string())
}

/// `<sha256 hex>  [*]<file name>` lines → (name, lowercase hex).
pub fn parse_checksums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (hash, rest) = line.split_at_checked(64)?;
            if !hash.bytes().all(|b| b.is_ascii_hexdigit()) || !rest.starts_with(char::is_whitespace) {
                return None;
            }
            let name = rest.trim_start();
            let name = name.strip_prefix('*').unwrap_or(name);
            (!name.is_empty()).then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

/// Checks a signed manifest for `version`/`archive` and returns the SHA-256
/// (lowercase hex) the archive must have: signature first, then the version
/// binding, then the entry.
pub fn verified_checksum(manifest: &[u8], sig_file: &[u8], keys: &[&str], version: &str, archive: &str) -> anyhow::Result<String> {
    verify_signature(manifest, sig_file, keys)?;
    let text = String::from_utf8_lossy(manifest);
    let declared = parse_manifest_version(&text);
    anyhow::ensure!(
        declared.as_deref() == Some(version),
        "version mismatch in the signed SHA256SUMS: asked for {version}, it declares {} — refusing (possible downgrade)",
        declared.as_deref().unwrap_or("none")
    );
    parse_checksums(&text)
        .into_iter()
        .find(|(name, _)| name == archive)
        .map(|(_, hash)| hash)
        .ok_or_else(|| anyhow::anyhow!("the signed SHA256SUMS has no entry for {archive}"))
}

/// The archive's SHA-256 matches the verified one.
pub fn check_archive_hash(expected: &str, actual: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected.eq_ignore_ascii_case(actual),
        "checksum mismatch for the CloakBrowser download (expected {expected}, got {actual}) — corrupted or tampered, refusing it"
    );
    Ok(())
}

// --------------------------------------------------------------- the build

/// Where a ready CloakBrowser binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `browser.cloak_path` in config.json.
    Configured,
    /// The host's cache (`~/.cache/unipi/cloakbrowser/<version>`).
    Cached,
    /// The wrapper's cache (`~/.cloakbrowser/chromium-<version>`).
    Wrapper,
    /// Downloaded and verified just now.
    Downloaded,
}

#[derive(Debug, Clone)]
pub struct CloakBinary {
    pub path: PathBuf,
    /// `None` for a configured binary (unknown).
    pub version: Option<String>,
    pub source: Source,
}

/// The version to use: `browser.cloak_version`, else this platform's pin.
pub fn wanted_version(configured: Option<&str>) -> anyhow::Result<(String, &'static str)> {
    let tag = platform_tag().ok_or_else(|| anyhow::anyhow!("CloakBrowser has no build for this platform ({} {})", std::env::consts::OS, std::env::consts::ARCH))?;
    let version = match configured.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => {
            anyhow::ensure!(valid_version(v), "browser.cloak_version {v:?} is not a full numeric Chromium version (e.g. 146.0.7680.177.5)");
            v.to_string()
        }
        None => pinned_version(tag).ok_or_else(|| anyhow::anyhow!("no free CloakBrowser build is pinned for {tag}"))?.to_string(),
    };
    Ok((version, tag))
}

/// A ready binary without downloading anything (status / `browser status`).
pub fn locate(cloak_path: Option<&str>, cloak_version: Option<&str>) -> anyhow::Result<Option<CloakBinary>> {
    if let Some(path) = cloak_path.map(str::trim).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        anyhow::ensure!(is_executable(&path), "browser.cloak_path {} is not an executable file", path.display());
        return Ok(Some(CloakBinary { path, version: None, source: Source::Configured }));
    }
    let (version, _) = wanted_version(cloak_version)?;
    if let Some(root) = cache_root() {
        let bin = binary_in(&root.join(&version));
        if is_executable(&bin) {
            return Ok(Some(CloakBinary { path: bin, version: Some(version), source: Source::Cached }));
        }
    }
    if let Some(root) = wrapper_cache_root() {
        let bin = binary_in(&root.join(format!("chromium-{version}")));
        if is_executable(&bin) {
            return Ok(Some(CloakBinary { path: bin, version: Some(version), source: Source::Wrapper }));
        }
    }
    Ok(None)
}

fn lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}

/// A ready CloakBrowser binary: configured, cached, the wrapper's, or
/// downloaded + verified now (one download at a time per process).
pub async fn ensure(cloak_path: Option<&str>, cloak_version: Option<&str>) -> anyhow::Result<CloakBinary> {
    if let Some(found) = locate(cloak_path, cloak_version)? {
        return Ok(found);
    }
    let _guard = lock().lock().await;
    if let Some(found) = locate(cloak_path, cloak_version)? {
        return Ok(found);
    }
    let (version, tag) = wanted_version(cloak_version)?;
    let root = cache_root().ok_or_else(|| anyhow::anyhow!("no cache directory ($HOME is not set)"))?;
    let dir = root.join(&version);
    download_and_install(&version, tag, &dir).await?;
    let bin = binary_in(&dir);
    anyhow::ensure!(is_executable(&bin), "the CloakBrowser {version} archive has no browser at {}", bin.display());
    Ok(CloakBinary { path: bin, version: Some(version), source: Source::Downloaded })
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .user_agent(concat!("unipi-host/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

async fn get_bytes(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let res = tokio::time::timeout(Duration::from_secs(30), client.get(url).send()).await.map_err(|_| anyhow::anyhow!("timed out"))??;
    anyhow::ensure!(res.status().is_success(), "HTTP {}", res.status());
    let bytes = tokio::time::timeout(Duration::from_secs(30), res.bytes()).await.map_err(|_| anyhow::anyhow!("timed out"))??;
    Ok(bytes.to_vec())
}

/// `SHA256SUMS` + `SHA256SUMS.sig` from one origin (primary, then GitHub).
async fn fetch_signed_manifest(client: &reqwest::Client, version: &str) -> anyhow::Result<(Vec<u8>, Vec<u8>, &'static str)> {
    let mut errors = Vec::new();
    for base in [DOWNLOAD_BASE_URL, GITHUB_DOWNLOAD_BASE_URL] {
        let manifest = match get_bytes(client, &release_url(base, version, "SHA256SUMS")).await {
            Ok(b) => b,
            Err(e) => {
                errors.push(format!("{base}: SHA256SUMS {e}"));
                continue;
            }
        };
        match get_bytes(client, &release_url(base, version, "SHA256SUMS.sig")).await {
            Ok(sig) => return Ok((manifest, sig, base)),
            Err(e) => errors.push(format!("{base}: SHA256SUMS.sig {e}")),
        }
    }
    anyhow::bail!("could not fetch a signed SHA256SUMS for CloakBrowser {version} — refusing an unverified binary ({})", errors.join("; "))
}

/// Streams `url` into `dest`, returning its SHA-256 (hex). Logs progress.
async fn download(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<String> {
    use tokio::io::AsyncWriteExt;
    tracing::info!(op = "cloakbrowser", url, "downloading");
    let mut res = client.get(url).send().await?;
    anyhow::ensure!(res.status().is_success(), "HTTP {}", res.status());
    let total = res.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = sha2::Sha256::new();
    let (mut done, mut logged) = (0u64, 0u64);
    while let Some(chunk) = res.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if let Some(pct) = (done * 100).checked_div(total) {
            if pct >= logged + 10 {
                logged = pct - pct % 10;
                tracing::info!(op = "cloakbrowser", "download {pct}% ({} / {} MB)", done >> 20, total >> 20);
            }
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    anyhow::ensure!(total == 0 || done == total, "download ended early ({done} of {total} bytes)");
    tracing::info!(op = "cloakbrowser", mb = done >> 20, "download complete");
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

async fn download_and_install(version: &str, tag: &str, dir: &Path) -> anyhow::Result<()> {
    let parent = dir.parent().ok_or_else(|| anyhow::anyhow!("bad cache dir"))?.to_path_buf();
    tokio::fs::create_dir_all(&parent).await?;
    let client = client()?;
    let archive = archive_name(tag);
    tracing::info!(op = "cloakbrowser", version, platform = tag, "CloakBrowser not cached: fetching the signed checksums");
    let (manifest, sig, origin) = fetch_signed_manifest(&client, version).await?;
    let expected = verified_checksum(&manifest, &sig, SIGNING_PUBKEYS, version, &archive)?;
    tracing::info!(op = "cloakbrowser", origin, "SHA256SUMS signature verified (Ed25519)");
    let tmp = parent.join(format!("_download_{:016x}_{archive}", rand::random::<u64>()));
    struct Remove(PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Remove(tmp.clone());
    let mut got = None;
    let mut errors = Vec::new();
    for base in [DOWNLOAD_BASE_URL, GITHUB_DOWNLOAD_BASE_URL] {
        match download(&client, &release_url(base, version, &archive), &tmp).await {
            Ok(hash) => {
                got = Some(hash);
                break;
            }
            Err(e) => {
                tracing::warn!(op = "cloakbrowser", base, error = %e, "download failed");
                errors.push(format!("{base}: {e}"));
            }
        }
    }
    let got = got.ok_or_else(|| anyhow::anyhow!("could not download CloakBrowser {version} ({})", errors.join("; ")))?;
    check_archive_hash(&expected, &got)?;
    tracing::info!(op = "cloakbrowser", "checksum verified (SHA-256): extracting");
    let (tmp2, dir2) = (tmp.clone(), dir.to_path_buf());
    tokio::task::spawn_blocking(move || install(&tmp2, &dir2)).await??;
    tracing::info!(op = "cloakbrowser", dir = %dir.display(), "CloakBrowser ready");
    Ok(())
}

/// Extracts a verified archive into a private staging dir next to `dir`,
/// then renames it into place (an existing complete install wins).
pub fn install(archive: &Path, dir: &Path) -> anyhow::Result<()> {
    let parent = dir.parent().ok_or_else(|| anyhow::anyhow!("bad cache dir"))?;
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("build");
    let staging = parent.join(format!("{name}.partial-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> anyhow::Result<()> {
        extract(archive, &staging)?;
        flatten_single_subdir(&staging)?;
        let bin = binary_in(&staging);
        anyhow::ensure!(bin.is_file(), "the archive has no {}", bin.strip_prefix(&staging).unwrap_or(&bin).display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
        }
        if cfg!(target_os = "macos") {
            let _ = std::process::Command::new("xattr").arg("-cr").arg(&staging).status();
        }
        for _ in 0..3 {
            match std::fs::rename(&staging, dir) {
                Ok(()) => return Ok(()),
                Err(_) if is_executable(&binary_in(dir)) => return Ok(()),
                Err(e) if !dir.exists() => return Err(e.into()),
                Err(_) => {
                    // An incomplete leftover: move it aside, then retry.
                    let stale = parent.join(format!("{name}.stale-{:016x}", rand::random::<u64>()));
                    std::fs::rename(dir, &stale)?;
                    let _ = std::fs::remove_dir_all(&stale);
                }
            }
        }
        anyhow::bail!("could not move the CloakBrowser build into {}", dir.display())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn extract(archive: &Path, into: &Path) -> anyhow::Result<()> {
    if archive.extension().is_some_and(|e| e == "zip") {
        // Windows 10+ ships bsdtar, which reads zip.
        let status = std::process::Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(into).status()?;
        anyhow::ensure!(status.success(), "tar -xf failed ({status})");
        return Ok(());
    }
    let file = std::fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    tar.set_preserve_permissions(true);
    tar.set_unpack_xattrs(false);
    tar.unpack(into)?;
    Ok(())
}

/// A tarball wrapping everything in one top-level directory: move its
/// contents up (never a macOS `.app` bundle).
fn flatten_single_subdir(dir: &Path) -> std::io::Result<()> {
    let entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    if entries.len() != 1 {
        return Ok(());
    }
    let only = entries[0].path();
    if !only.is_dir() || only.extension().is_some_and(|e| e == "app") {
        return Ok(());
    }
    for child in std::fs::read_dir(&only)?.flatten() {
        std::fs::rename(child.path(), dir.join(child.file_name()))?;
    }
    std::fs::remove_dir(&only)
}

// --------------------------------------------------------------- the seed

fn read_seed(file: &Path) -> std::io::Result<Option<u32>> {
    let text = match std::fs::read(file) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match text.parse::<u32>() {
        Ok(seed) if (SEED_MIN..=SEED_MAX).contains(&seed) && text.bytes().all(|b| b.is_ascii_digit()) => Ok(Some(seed)),
        _ => {
            tracing::warn!(op = "cloakbrowser", file = %file.display(), "corrupt fingerprint seed file: generating a new seed (new identity)");
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(None),
            }
        }
    }
}

/// The profile's fingerprint seed from `<profile>/.cloakbrowser-seed`,
/// created once (never overwritten: concurrent first launches agree).
pub fn profile_seed(profile: &Path) -> std::io::Result<u32> {
    use std::io::Write;
    let file = profile.join(SEED_FILE);
    if let Some(seed) = read_seed(&file)? {
        return Ok(seed);
    }
    std::fs::create_dir_all(profile)?;
    let content = format!("{}\n", rand::random_range(SEED_MIN..=SEED_MAX));
    let tmp = profile.join(format!("{SEED_FILE}.{:016x}.tmp", rand::random::<u64>()));
    let write_new = |path: &Path| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()
    };
    let published = write_new(&tmp).and_then(|()| match std::fs::hard_link(&tmp, &file) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => match write_new(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            other => other,
        },
        Ok(()) => Ok(()),
    });
    let _ = std::fs::remove_file(&tmp);
    published?;
    read_seed(&file)?.ok_or_else(|| std::io::Error::other(format!("could not create {}", file.display())))
}

/// The wrapper's stealth flags for a headed persistent launch with `seed`:
/// `--no-sandbox`, the fingerprint seed, the spoofed platform (Windows on
/// Linux/Windows, native macOS on a Mac) and `--ignore-gpu-blocklist`
/// (headed: lets SwiftShader serve WebGL on software GPUs).
pub fn stealth_args(seed: u32) -> Vec<String> {
    let platform = if cfg!(target_os = "macos") { "macos" } else { "windows" };
    vec![
        "--no-sandbox".into(),
        format!("--fingerprint={seed}"),
        format!("--fingerprint-platform={platform}"),
        "--ignore-gpu-blocklist".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real release manifest + signature of 146.0.7680.177.5 (fetched
    // from cloakbrowser.dev, UNI-292).
    const MANIFEST: &str = "version=146.0.7680.177.5\n4a12bcde95fa1bb1beef2b41ab5e5c27c36be78e3be3d0dac8c64d705216670e  cloakbrowser-linux-x64.tar.gz\nb213795cb32c3169f766c74ce1d0275fc89d3df256de39c04da7fb4c23b7fdbe  cloakbrowser-windows-x64.zip\n";
    const SIG: &str = "7a5X5wZvMJXQQcJjj0d1WNb5dDkL2ze4X8vV26P+Cl7UFpVdO8xlLcmMK1tbpqx3ExLEnw72bvL2UBiWSwtzBQ==\n";

    fn test_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
    }
    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
    fn sign(key: &ed25519_dalek::SigningKey, msg: &[u8]) -> String {
        use ed25519_dalek::Signer;
        b64(&key.sign(msg).to_bytes())
    }

    #[test]
    fn the_real_release_verifies_with_the_pinned_key() {
        verify_signature(MANIFEST.as_bytes(), SIG.as_bytes(), SIGNING_PUBKEYS).unwrap();
        let hash = verified_checksum(MANIFEST.as_bytes(), SIG.as_bytes(), SIGNING_PUBKEYS, "146.0.7680.177.5", "cloakbrowser-linux-x64.tar.gz").unwrap();
        assert_eq!(hash, "4a12bcde95fa1bb1beef2b41ab5e5c27c36be78e3be3d0dac8c64d705216670e");
    }

    #[test]
    fn a_tampered_manifest_is_refused() {
        let tampered = MANIFEST.replace("4a12bcde", "4a12bcdf");
        let e = verify_signature(tampered.as_bytes(), SIG.as_bytes(), SIGNING_PUBKEYS).unwrap_err();
        assert!(e.to_string().contains("signature verification failed"), "{e}");
        // One flipped bit of the signature, too.
        let mut sig = base64::engine::general_purpose::STANDARD.decode(SIG.trim()).unwrap();
        sig[10] ^= 1;
        assert!(verify_signature(MANIFEST.as_bytes(), b64(&sig).as_bytes(), SIGNING_PUBKEYS).is_err());
        // Malformed signature files.
        assert!(verify_signature(MANIFEST.as_bytes(), b"not base64!!", SIGNING_PUBKEYS).unwrap_err().to_string().contains("malformed"));
        assert!(verify_signature(MANIFEST.as_bytes(), b64(&[1u8; 63]).as_bytes(), SIGNING_PUBKEYS).unwrap_err().to_string().contains("malformed"));
    }

    #[test]
    fn the_wrong_key_is_refused_and_rotation_works() {
        let other = b64(test_key().verifying_key().as_bytes());
        // The real signature against a key it wasn't made with.
        assert!(verify_signature(MANIFEST.as_bytes(), SIG.as_bytes(), &[other.as_str()]).is_err());
        // A manifest signed by some other key, checked against the pinned key.
        let forged = sign(&test_key(), MANIFEST.as_bytes());
        assert!(verify_signature(MANIFEST.as_bytes(), forged.as_bytes(), SIGNING_PUBKEYS).is_err());
        // Any of several pinned keys may validate; garbage keys are skipped.
        verify_signature(MANIFEST.as_bytes(), forged.as_bytes(), &["garbage", SIGNING_PUBKEYS[0], other.as_str()]).unwrap();
    }

    #[test]
    fn version_binding_and_entry_and_hash() {
        let key = test_key();
        let pubkey = b64(key.verifying_key().as_bytes());
        let keys = [pubkey.as_str()];
        let old = "version=145.0.0.1\naaaa000000000000000000000000000000000000000000000000000000000000  cloakbrowser-linux-x64.tar.gz\n";
        let sig = sign(&key, old.as_bytes());
        // A genuinely signed OLDER release served for the version we asked for.
        let e = verified_checksum(old.as_bytes(), sig.as_bytes(), &keys, "146.0.7680.177.5", "cloakbrowser-linux-x64.tar.gz").unwrap_err();
        assert!(e.to_string().contains("possible downgrade"), "{e}");
        let e = verified_checksum(old.as_bytes(), sig.as_bytes(), &keys, "145.0.0.1", "cloakbrowser-darwin-x64.tar.gz").unwrap_err();
        assert!(e.to_string().contains("no entry"), "{e}");
        let hash = verified_checksum(old.as_bytes(), sig.as_bytes(), &keys, "145.0.0.1", "cloakbrowser-linux-x64.tar.gz").unwrap();
        check_archive_hash(&hash, &hash.to_uppercase()).unwrap();
        assert!(check_archive_hash(&hash, &"b".repeat(64)).unwrap_err().to_string().contains("checksum mismatch"));
        // No version line at all.
        let bare = "aaaa000000000000000000000000000000000000000000000000000000000000  x\n";
        let sig = sign(&key, bare.as_bytes());
        assert!(verified_checksum(bare.as_bytes(), sig.as_bytes(), &keys, "1.2.3.4", "x").is_err());
    }

    #[test]
    fn checksum_lines_parse_like_the_wrapper() {
        let parsed = parse_checksums("version=1.2.3.4\nABCDEF0000000000000000000000000000000000000000000000000000000000 *a.zip\nzz  b\n\n0000000000000000000000000000000000000000000000000000000000000000  b.tar.gz\n");
        assert_eq!(
            parsed,
            vec![("a.zip".to_string(), format!("abcdef{}", "0".repeat(58))), ("b.tar.gz".to_string(), "0".repeat(64))]
        );
        assert_eq!(parse_manifest_version(MANIFEST).as_deref(), Some("146.0.7680.177.5"));
    }

    #[test]
    fn versions_urls_and_names() {
        assert!(valid_version("146.0.7680.177.5") && valid_version("146.0.7680.177"));
        for bad in ["146", "146.0.7680", "146.0.7680.177.5.1", "146.0.x.1", "", "1..2.3"] {
            assert!(!valid_version(bad), "{bad}");
        }
        assert_eq!(pinned_version("linux-x64"), Some("146.0.7680.177.5"));
        assert_eq!(archive_name("linux-x64"), "cloakbrowser-linux-x64.tar.gz");
        assert_eq!(archive_name("windows-x64"), "cloakbrowser-windows-x64.zip");
        assert_eq!(
            release_url(DOWNLOAD_BASE_URL, "146.0.7680.177.5", "SHA256SUMS"),
            "https://cloakbrowser.dev/chromium-v146.0.7680.177.5/SHA256SUMS"
        );
        if cfg!(target_os = "linux") {
            assert_eq!(binary_in(Path::new("/c/146")), Path::new("/c/146/chrome"));
        }
    }

    #[test]
    fn seed_is_created_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("profile");
        let seed = profile_seed(&p).unwrap();
        assert!((SEED_MIN..=SEED_MAX).contains(&seed));
        assert_eq!(std::fs::read_to_string(p.join(SEED_FILE)).unwrap(), format!("{seed}\n"));
        assert_eq!(profile_seed(&p).unwrap(), seed);
        // Written by a wrapper (same format) → reused.
        std::fs::write(p.join(SEED_FILE), "54321\n").unwrap();
        assert_eq!(profile_seed(&p).unwrap(), 54321);
        // Corrupt / out of range → replaced.
        std::fs::write(p.join(SEED_FILE), "123").unwrap();
        let fresh = profile_seed(&p).unwrap();
        assert!((SEED_MIN..=SEED_MAX).contains(&fresh));
        // No temp files left behind.
        let names: Vec<String> = std::fs::read_dir(&p).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, [SEED_FILE]);
        let args = stealth_args(fresh);
        assert!(args.contains(&format!("--fingerprint={fresh}")) && args.contains(&"--no-sandbox".to_string()));
    }

    #[test]
    fn install_extracts_flattens_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        // A tarball like the release: one top-level dir with the binary.
        let archive = dir.path().join("cloakbrowser-test.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            let bin = binary_in(Path::new("cloakbrowser"));
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, &bin, &b"hello"[..]).unwrap();
            let mut h2 = tar::Header::new_gnu();
            h2.set_size(1);
            h2.set_mode(0o644);
            h2.set_cksum();
            tar.append_data(&mut h2, "cloakbrowser/resources.pak", &b"x"[..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let target = dir.path().join("cache").join("1.2.3.4");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        install(&archive, &target).unwrap();
        assert!(is_executable(&binary_in(&target)), "binary marked executable");
        assert!(target.join("resources.pak").is_file(), "flattened");
        // Again: the complete install stays.
        install(&archive, &target).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }
}
