//! Chrome-family cookie import (UNI-294): read a Chrome/Chromium/Brave
//! `Cookies` sqlite database for one domain (and its subdomains), decrypt
//! the encrypted values on Linux/macOS, and hand the rows to the serve to
//! inject into a CloakBrowser profile over CDP `Storage.setCookies`.
//!
//! **Never** prints, logs or returns a cookie value or a keyring secret:
//! only counts and domain names cross any boundary (CLI stdout, control.sock,
//! `unipi-profile.json`).
//!
//! ## Decryption (Linux; macOS below; Windows unsupported)
//!
//! Chrome's `encrypted_value` carries a 3-byte version prefix:
//!
//! - `v10`: AES-128-CBC, key = PBKDF2-HMAC-SHA1(password = the literal
//!   string `"peanuts"`, salt `"saltysalt"`, 1 iteration, 16 bytes), IV =
//!   16 spaces (`0x20`). This is what Chrome falls back to with no keyring
//!   (`--password-store=basic`, which is what the dedicated profile itself
//!   runs with, and what some desktop environments use for the user's
//!   Chrome too).
//! - `v11`: same cipher, but the PBKDF2 password is the user's own Secret
//!   Service secret (`org.freedesktop.secrets`, item attribute
//!   `application=chrome`/`chromium`/`brave`, label "<Browser> Safe
//!   Storage"), read with `secret-tool lookup application <name>` — the CLI
//!   process has the user's D-Bus session; the serve (no session, maybe a
//!   different user under systemd) never could.
//!
//! Decrypted plaintext for Chrome schema (`meta.version`) ≥ 24 carries a
//! 32-byte prefix (`SHA256(host_key)`, Chrome's own integrity check since
//! M119) before the UTF-8 value: stripped here, not verified (we trust our
//! own decryption).
//!
//! macOS reads the same secret from Keychain (`security find-generic-password
//! -w -s "Chrome Safe Storage"`) with 1003 PBKDF2 iterations instead of 1;
//! same cipher otherwise. Windows cookies (`v10`+AES-256-GCM+DPAPI, or the
//! App-Bound `v20`) are not supported yet — a clear error, no crash.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use aes::Aes128;
use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;

type Aes128CbcDec = cbc::Decryptor<Aes128>;

/// A browser kind we know how to find cookies for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Chrome,
    Chromium,
    Brave,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Chrome => "chrome",
            Source::Chromium => "chromium",
            Source::Brave => "brave",
        }
    }

    pub fn parse(text: &str) -> Option<Source> {
        match text.trim().to_ascii_lowercase().as_str() {
            "chrome" | "google-chrome" => Some(Source::Chrome),
            "chromium" => Some(Source::Chromium),
            "brave" => Some(Source::Brave),
            _ => None,
        }
    }

    /// The Secret Service / Keychain item attribute Chrome stores its
    /// `v11` secret under for this browser family.
    pub fn keyring_application(self) -> &'static str {
        match self {
            Source::Chrome => "chrome",
            Source::Chromium => "chromium",
            Source::Brave => "brave",
        }
    }

    /// Keychain service name (macOS).
    pub fn keychain_service(self) -> &'static str {
        match self {
            Source::Chrome => "Chrome Safe Storage",
            Source::Chromium => "Chromium Safe Storage",
            Source::Brave => "Brave Safe Storage",
        }
    }

    /// The default `User Data` directory per OS, newest-looking first
    /// (the caller picks the first that exists).
    pub fn user_data_dirs(self, home: &Path) -> Vec<PathBuf> {
        if cfg!(target_os = "macos") {
            let base = home.join("Library/Application Support");
            match self {
                Source::Chrome => vec![base.join("Google/Chrome")],
                Source::Chromium => vec![base.join("Chromium")],
                Source::Brave => vec![base.join("BraveSoftware/Brave-Browser")],
            }
        } else if cfg!(windows) {
            vec![] // caller falls back to %LOCALAPPDATA% via from_dir / env
        } else {
            let cfg = home.join(".config");
            match self {
                Source::Chrome => vec![cfg.join("google-chrome"), cfg.join("google-chrome-beta")],
                Source::Chromium => vec![cfg.join("chromium")],
                Source::Brave => vec![cfg.join("BraveSoftware/Brave-Browser")],
            }
        }
    }
}

/// One importable (browser, profile) pair detected on this machine;
/// nothing secret — a name and a path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedSource {
    pub browser: Source,
    /// The `User Data` directory (e.g. `~/.config/google-chrome`).
    pub user_data_dir: String,
    /// Profile directory names inside it with a `Cookies` database
    /// (`Default`, `Profile 1`, …).
    pub profiles: Vec<String>,
}

/// A profile directory has a readable (not necessarily non-empty) cookies db.
fn has_cookies_db(profile_dir: &Path) -> bool {
    cookies_db_path(profile_dir).is_some()
}

/// `<profile>/Network/Cookies` (modern Chrome) or `<profile>/Cookies` (older).
pub fn cookies_db_path(profile_dir: &Path) -> Option<PathBuf> {
    [profile_dir.join("Network").join("Cookies"), profile_dir.join("Cookies")].into_iter().find(|c| c.is_file())
}

/// Profile directory names (`Default`, `Profile 1`, …) inside a `User Data`
/// dir that have a cookies database, sorted with `Default` first.
pub fn profiles_in(user_data_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(user_data_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| (n == "Default" || n.starts_with("Profile ")) && has_cookies_db(&user_data_dir.join(n)))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by(|a, b| (a != "Default", a).cmp(&(b != "Default", b)));
    names
}

/// A `--from`/`--from-dir` spec resolved to one cookies database.
pub struct ResolvedSource {
    pub db_path: PathBuf,
    /// `None` for `--from-dir` (we don't know which brand it is; no `v11`
    /// keyring lookup is attempted — `--from` is for that).
    pub browser: Option<Source>,
}

/// `chrome[:<profile dir>]` / `chromium[:…]` / `brave[:…]` → the browser's
/// standard `User Data` dir (newest that exists) + the named or newest
/// profile. `None` profile = the first of [`profiles_in`] (`Default` first).
pub fn resolve_from(spec: &str, home: &Path) -> Result<ResolvedSource, String> {
    let (name, profile) = spec.split_once(':').map(|(n, p)| (n, Some(p))).unwrap_or((spec, None));
    let browser = Source::parse(name).ok_or_else(|| format!("unknown browser {name:?} (chrome, chromium, brave)"))?;
    let user_data_dir = browser
        .user_data_dirs(home)
        .into_iter()
        .find(|d| d.is_dir())
        .ok_or_else(|| format!("no {name} profile directory found on this PC"))?;
    let profile = match profile {
        Some(p) => p.to_string(),
        None => profiles_in(&user_data_dir).into_iter().next().ok_or_else(|| format!("no importable {name} profile (no Cookies database)"))?,
    };
    let db_path = cookies_db_path(&user_data_dir.join(&profile))
        .ok_or_else(|| format!("no Cookies database in {} profile {profile:?}", user_data_dir.display()))?;
    Ok(ResolvedSource { db_path, browser: Some(browser) })
}

/// `<user data dir>[:<profile dir>]`. No browser guess (and so no `v11`
/// keyring secret attempt) — meant for a throwaway/test Chromium profile
/// without one, or a non-standard install.
pub fn resolve_from_dir(spec: &str) -> Result<ResolvedSource, String> {
    let (dir, profile) = spec.split_once(':').map(|(d, p)| (d, Some(p))).unwrap_or((spec, None));
    let user_data_dir = PathBuf::from(dir);
    if !user_data_dir.is_dir() {
        return Err(format!("{} is not a directory", user_data_dir.display()));
    }
    let profile = match profile {
        Some(p) => p.to_string(),
        None => profiles_in(&user_data_dir).into_iter().next().ok_or_else(|| format!("no importable profile under {}", user_data_dir.display()))?,
    };
    let db_path = cookies_db_path(&user_data_dir.join(&profile))
        .ok_or_else(|| format!("no Cookies database in {} profile {profile:?}", user_data_dir.display()))?;
    Ok(ResolvedSource { db_path, browser: None })
}

/// Every Chrome-family browser with at least one importable profile on this
/// machine (`--list-sources`). Detection only — reads no secrets.
pub fn detect_sources(home: &Path) -> Vec<DetectedSource> {
    [Source::Chrome, Source::Chromium, Source::Brave]
        .into_iter()
        .filter_map(|browser| {
            browser.user_data_dirs(home).into_iter().find(|d| d.is_dir()).and_then(|dir| {
                let profiles = profiles_in(&dir);
                (!profiles.is_empty())
                    .then(|| DetectedSource { browser, user_data_dir: dir.display().to_string(), profiles })
            })
        })
        .collect()
}

// ------------------------------------------------------------ decryption

/// PBKDF2-HMAC-SHA1(secret, "saltysalt", iterations, 16 bytes) — Chrome's
/// own key derivation for both `v10` ("peanuts") and `v11` (the keyring
/// secret), Linux 1 iteration, macOS 1003.
pub fn derive_key(secret: &[u8], iterations: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(secret, b"saltysalt", iterations, &mut key);
    key
}

/// AES-128-CBC, IV = 16 spaces, PKCS7 padding — Chrome's cipher for `v10`/`v11`.
pub fn aes_cbc_decrypt(key: &[u8; 16], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let iv = [0x20u8; 16];
    let mut buf = ciphertext.to_vec();
    let decryptor = Aes128CbcDec::new(key.into(), &iv.into());
    decryptor
        .decrypt_padded::<Pkcs7>(&mut buf)
        .map(<[u8]>::to_vec)
        .map_err(|_| "bad padding (wrong key or truncated value)".to_string())
}

/// Chrome ≥ schema 24 prepends `SHA256(host_key)` (32 bytes) to the
/// plaintext; strip it (we don't re-verify it — our own decryption).
pub fn strip_meta_prefix(plaintext: Vec<u8>, meta_version: i64) -> Vec<u8> {
    if meta_version >= 24 && plaintext.len() >= 32 {
        plaintext[32..].to_vec()
    } else {
        plaintext
    }
}

/// Decrypts one `encrypted_value` cell: `v10` with `peanuts`, `v11` with
/// `keyring_secret` (required then). Anything else (empty, `v20`, garbage)
/// is refused with a short reason (never the bytes).
pub fn decrypt_cookie_value(
    encrypted: &[u8],
    meta_version: i64,
    iterations: u32,
    keyring_secret: Option<&[u8]>,
) -> Result<String, String> {
    if encrypted.len() < 3 {
        return Err("too short".into());
    }
    let (prefix, body) = (&encrypted[..3], &encrypted[3..]);
    let secret: &[u8] = match prefix {
        b"v10" => b"peanuts",
        b"v11" => keyring_secret.ok_or("v11 cookie but no keyring secret available")?,
        other => return Err(format!("unsupported encryption prefix {:?}", String::from_utf8_lossy(other))),
    };
    let key = derive_key(secret, iterations);
    let plaintext = aes_cbc_decrypt(&key, body)?;
    let plaintext = strip_meta_prefix(plaintext, meta_version);
    String::from_utf8(plaintext).map_err(|_| "decrypted value is not UTF-8 (wrong key?)".to_string())
}

/// Reads the Secret Service secret with `secret-tool` (Linux). Run in the
/// CLI process, which has the user's D-Bus session; the serve never does.
pub fn secret_tool_lookup(application: &str) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "application", application])
        .output()
        .map_err(|e| format!("could not run secret-tool: {e}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(format!(
            "no Secret Service secret for application={application} (is a keyring unlocked? `secret-tool lookup application {application}`)"
        ));
    }
    Ok(output.stdout)
}

/// Reads the Keychain secret with `security` (macOS).
#[cfg(target_os = "macos")]
pub fn keychain_lookup(service: &str) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", service])
        .output()
        .map_err(|e| format!("could not run security: {e}"))?;
    if !output.status.success() {
        return Err(format!("no Keychain item {service:?} (denied, or not this account's Chrome)"));
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    while text.ends_with('\n') {
        text.pop();
    }
    Ok(text.into_bytes())
}

/// PBKDF2 iterations for the platform (1 Linux, 1003 macOS).
pub fn pbkdf2_iterations() -> u32 {
    if cfg!(target_os = "macos") {
        1003
    } else {
        1
    }
}

// ------------------------------------------------------------------ rows

/// One decrypted cookie, ready for CDP `Storage.setCookies` / a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<&'static str>,
    /// Unix seconds; `None` = session cookie (no explicit expiry sent).
    pub expires: Option<i64>,
}

/// Chrome's `samesite` column (`-1` unset, `0` none, `1` lax, `2` strict) →
/// CDP's `Strict`/`Lax`/`None` (CDP has no "unspecified"; Chrome's own
/// cookie jar defaults unset cookies to `Lax` the same way).
pub fn map_samesite(value: i64) -> &'static str {
    match value {
        0 => "None",
        2 => "Strict",
        _ => "Lax",
    }
}

/// Chrome's `expires_utc` (microseconds since 1601-01-01) → unix seconds;
/// `0` (session cookie) → `None`.
pub fn chrome_time_to_unix(expires_utc: i64) -> Option<i64> {
    const EPOCH_DELTA_US: i64 = 11_644_473_600_000_000; // 1601→1970, in µs
    if expires_utc == 0 {
        None
    } else {
        Some((expires_utc - EPOCH_DELTA_US) / 1_000_000)
    }
}

/// Does a cookie's `host_key` (as Chrome stores it, `.example.com` or
/// `sub.example.com`) belong to `domain` (itself or a subdomain)?
pub fn host_matches(host_key: &str, domain: &str) -> bool {
    let host = host_key.trim_start_matches('.');
    let domain = domain.trim_start_matches('.');
    host.eq_ignore_ascii_case(domain) || host.to_ascii_lowercase().ends_with(&format!(".{}", domain.to_ascii_lowercase()))
}

/// One row straight off the `cookies` table.
struct RawRow {
    host_key: String,
    name: String,
    path: String,
    expires_utc: i64,
    is_secure: i64,
    is_httponly: i64,
    samesite: i64,
    value: String,
    encrypted_value: Vec<u8>,
}

/// Reads `meta.version` and every `cookies` row whose `host_key` matches
/// `domain` (itself or a subdomain) from a sqlite file at `path` (meant to
/// be a private copy, never the live db).
fn read_rows(path: &Path, domain: &str) -> Result<(i64, Vec<RawRow>), String> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("could not open the cookie database: {e}"))?;
    let meta_version: i64 = conn
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut stmt = conn
        .prepare("SELECT host_key, name, path, expires_utc, is_secure, is_httponly, samesite, value, encrypted_value FROM cookies")
        .map_err(|e| format!("could not read the cookies table: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RawRow {
                host_key: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                expires_utc: r.get(3)?,
                is_secure: r.get(4)?,
                is_httponly: r.get(5)?,
                samesite: r.get(6)?,
                value: r.get(7)?,
                encrypted_value: r.get(8)?,
            })
        })
        .map_err(|e| format!("could not read the cookies table: {e}"))?
        .filter_map(|r| r.ok())
        .filter(|r| host_matches(&r.host_key, domain))
        .collect();
    Ok((meta_version, rows))
}

/// Imported / skipped counts, and the decrypted rows (consumed once by the
/// caller before being dropped — never logged).
pub struct ImportedCookies {
    pub cookies: Vec<DecryptedCookie>,
    pub skipped: usize,
}

/// Copies `db_path` (and its `-wal`/`-shm`, if present) into a fresh temp
/// dir, reads the rows for `domain`, decrypts them (v10 always; v11 needs
/// `keyring_secret`), and removes the temp copy. Never touches the live db
/// beyond the initial copy.
pub fn import_from_db(db_path: &Path, domain: &str, keyring_secret: Option<&[u8]>) -> Result<ImportedCookies, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("could not create a temp dir: {e}"))?;
    let copy = dir.path().join("Cookies");
    std::fs::copy(db_path, &copy).map_err(|e| format!("could not copy the cookie database: {e}"))?;
    for ext in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{ext}", db_path.display()));
        if side.is_file() {
            let _ = std::fs::copy(&side, PathBuf::from(format!("{}{ext}", copy.display())));
        }
    }
    let (meta_version, rows) = read_rows(&copy, domain)?;
    let iterations = pbkdf2_iterations();
    let mut cookies = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let value = if !row.value.is_empty() {
            row.value
        } else if row.encrypted_value.is_empty() {
            skipped += 1;
            continue;
        } else {
            match decrypt_cookie_value(&row.encrypted_value, meta_version, iterations, keyring_secret) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
        };
        cookies.push(DecryptedCookie {
            name: row.name,
            value,
            domain: row.host_key,
            path: if row.path.is_empty() { "/".to_string() } else { row.path },
            secure: row.is_secure != 0,
            http_only: row.is_httponly != 0,
            same_site: Some(map_samesite(row.samesite)),
            expires: chrome_time_to_unix(row.expires_utc),
        });
    }
    Ok(ImportedCookies { cookies, skipped })
}

/// The keyring secret for `browser`'s `v11` cookies on this platform
/// (Linux: Secret Service via `secret-tool`; macOS: Keychain). Run in the
/// CLI process, which has the session; `None` on Windows (unsupported) or
/// when nothing is configured (then only `v10`/plaintext cookies import).
pub fn keyring_secret(browser: Source) -> Option<Vec<u8>> {
    if cfg!(windows) {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        keychain_lookup(browser.keychain_service()).ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        secret_tool_lookup(browser.keyring_application()).ok()
    }
}

/// Resolves the keyring secret (when `browser` is known) and imports
/// `domain`'s cookies from `db_path`.
pub fn run_import(db_path: &Path, browser: Option<Source>, domain: &str) -> Result<ImportedCookies, String> {
    let secret = browser.and_then(keyring_secret);
    import_from_db(db_path, domain, secret.as_deref())
}

/// `Storage.setCookies`' `cookies` array for one [`DecryptedCookie`].
pub fn to_cdp_cookie(cookie: &DecryptedCookie) -> serde_json::Value {
    let mut value = serde_json::json!({
        "name": cookie.name,
        "value": cookie.value,
        "domain": cookie.domain,
        "path": cookie.path,
        "secure": cookie.secure,
        "httpOnly": cookie.http_only,
    });
    if let Some(same_site) = cookie.same_site {
        value["sameSite"] = same_site.into();
    }
    if let Some(expires) = cookie.expires {
        value["expires"] = expires.into();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db(dir: &Path, meta_version: i64) -> PathBuf {
        let path = dir.join("Cookies");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT, value TEXT);
             CREATE TABLE cookies (
                host_key TEXT, name TEXT, path TEXT, expires_utc INTEGER,
                is_secure INTEGER, is_httponly INTEGER, samesite INTEGER,
                value TEXT, encrypted_value BLOB);",
        )
        .unwrap();
        conn.execute("INSERT INTO meta VALUES ('version', ?1)", [meta_version.to_string()]).unwrap();
        path
    }

    fn encrypt_v10(secret: &[u8], iterations: u32, plaintext: &[u8], meta_version: i64) -> Vec<u8> {
        let key = derive_key(secret, iterations);
        let prefixed = if meta_version >= 24 {
            let mut v = vec![0u8; 32];
            v.extend_from_slice(plaintext);
            v
        } else {
            plaintext.to_vec()
        };
        let iv = [0x20u8; 16];
        let encryptor = cbc::Encryptor::<Aes128>::new(&key.into(), &iv.into());
        use aes::cipher::BlockModeEncrypt;
        let mut buf = vec![0u8; prefixed.len() + 16];
        buf[..prefixed.len()].copy_from_slice(&prefixed);
        let ct = encryptor.encrypt_padded::<Pkcs7>(&mut buf, prefixed.len()).unwrap();
        let mut out = b"v10".to_vec();
        out.extend_from_slice(ct);
        out
    }

    #[test]
    fn v10_roundtrip_with_the_hardcoded_password() {
        let ct = encrypt_v10(b"peanuts", 1, b"abc123", 0);
        let plain = decrypt_cookie_value(&ct, 0, 1, None).unwrap();
        assert_eq!(plain, "abc123");
    }

    #[test]
    fn v11_needs_the_keyring_secret_and_fails_without_it() {
        let secret = b"my-keyring-secret";
        let ct_v11 = {
            let mut bytes = encrypt_v10(secret, 1, b"session-token", 0);
            bytes[..3].copy_from_slice(b"v11");
            bytes
        };
        assert!(decrypt_cookie_value(&ct_v11, 0, 1, None).is_err());
        let plain = decrypt_cookie_value(&ct_v11, 0, 1, Some(secret)).unwrap();
        assert_eq!(plain, "session-token");
        // The wrong secret: garbage or a padding error, never a crash.
        assert!(decrypt_cookie_value(&ct_v11, 0, 1, Some(b"wrong")).is_err());
    }

    #[test]
    fn meta_version_24_prefix_is_stripped() {
        let ct = encrypt_v10(b"peanuts", 1, b"value-after-prefix", 24);
        let plain = decrypt_cookie_value(&ct, 24, 1, None).unwrap();
        assert_eq!(plain, "value-after-prefix");
        // Without the version bump the 32 leading bytes would stay in.
        let ct_old = encrypt_v10(b"peanuts", 1, b"value-after-prefix", 0);
        let plain_old = decrypt_cookie_value(&ct_old, 0, 1, None).unwrap();
        assert_eq!(plain_old, "value-after-prefix");
    }

    #[test]
    fn unsupported_prefixes_and_short_values_are_refused() {
        assert!(decrypt_cookie_value(b"v2", 0, 1, None).is_err());
        assert!(decrypt_cookie_value(b"v20garbage", 0, 1, None).is_err());
        assert!(decrypt_cookie_value(b"", 0, 1, None).is_err());
    }

    #[test]
    fn host_matches_domain_and_subdomains_only() {
        assert!(host_matches(".example.com", "example.com"));
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("sub.example.com", "example.com"));
        assert!(host_matches(".sub.example.com", "example.com"));
        assert!(!host_matches("notexample.com", "example.com"));
        assert!(!host_matches("example.com.evil.com", "example.com"));
        assert!(!host_matches("example.org", "example.com"));
    }

    #[test]
    fn samesite_and_time_mapping() {
        assert_eq!(map_samesite(0), "None");
        assert_eq!(map_samesite(1), "Lax");
        assert_eq!(map_samesite(2), "Strict");
        assert_eq!(map_samesite(-1), "Lax");
        assert_eq!(chrome_time_to_unix(0), None);
        // 2026-01-01T00:00:00Z in Chrome microseconds-since-1601.
        let unix_2026 = 1_767_225_600i64;
        let chrome_us = unix_2026 * 1_000_000 + 11_644_473_600_000_000;
        assert_eq!(chrome_time_to_unix(chrome_us), Some(unix_2026));
    }

    #[test]
    fn import_from_a_synthetic_db_v10_and_v11_mixed_and_meta24() {
        let dir = tempfile::tempdir().unwrap();
        let db = test_db(dir.path(), 24);
        let keyring_secret = b"the-keyring-secret";
        let conn = rusqlite::Connection::open(&db).unwrap();
        let insert = |host: &str, name: &str, enc: &[u8], plain_value: &str| {
            conn.execute(
                "INSERT INTO cookies (host_key, name, path, expires_utc, is_secure, is_httponly, samesite, value, encrypted_value) VALUES (?1, ?2, '/', 0, 1, 0, 2, ?3, ?4)",
                rusqlite::params![host, name, plain_value, enc],
            )
            .unwrap();
        };
        // v10 cookie for the domain, meta 24 prefix.
        insert("example.com", "sid", &encrypt_v10(b"peanuts", 1, b"tok1", 24), "");
        // v11 cookie for a subdomain, same meta version.
        let mut v11 = encrypt_v10(keyring_secret, 1, b"tok2", 24);
        v11[..3].copy_from_slice(b"v11");
        insert("sub.example.com", "auth", &v11, "");
        // A plaintext cookie (no decryption needed).
        insert(".example.com", "plain", b"", "plainvalue");
        // A cookie for a different domain: not selected.
        insert("other.com", "ignored", &encrypt_v10(b"peanuts", 1, b"nope", 24), "");
        // A v11 cookie we can't decrypt without the secret: counted as skipped.
        let mut v11_locked = encrypt_v10(b"not-the-secret", 1, b"locked", 24);
        v11_locked[..3].copy_from_slice(b"v11");
        insert("example.com", "locked", &v11_locked, "");

        let imported = import_from_db(&db, "example.com", Some(keyring_secret)).unwrap();
        let mut names: Vec<&str> = imported.cookies.iter().map(|c| c.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["auth", "plain", "sid"]);
        assert_eq!(imported.skipped, 1, "the wrong-secret v11 cookie");
        let sid = imported.cookies.iter().find(|c| c.name == "sid").unwrap();
        assert_eq!((sid.value.as_str(), sid.domain.as_str(), sid.secure, sid.same_site), ("tok1", "example.com", true, Some("Strict")));

        // Without the keyring secret: the v11 cookie is skipped too, the rest import.
        let imported = import_from_db(&db, "example.com", None).unwrap();
        let mut names: Vec<&str> = imported.cookies.iter().map(|c| c.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["plain", "sid"]);
        assert_eq!(imported.skipped, 2);
    }

    #[test]
    fn to_cdp_cookie_shape() {
        let cookie = DecryptedCookie {
            name: "sid".into(),
            value: "tok".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: Some("Lax"),
            expires: Some(1_700_000_000),
        };
        let v = to_cdp_cookie(&cookie);
        assert_eq!(v["name"], "sid");
        assert_eq!(v["value"], "tok");
        assert_eq!(v["domain"], "example.com");
        assert_eq!(v["secure"], true);
        assert_eq!(v["httpOnly"], true);
        assert_eq!(v["sameSite"], "Lax");
        assert_eq!(v["expires"], 1_700_000_000);
    }

    #[test]
    fn source_parsing_and_keyring_names() {
        assert_eq!(Source::parse("Chrome"), Some(Source::Chrome));
        assert_eq!(Source::parse("google-chrome"), Some(Source::Chrome));
        assert_eq!(Source::parse("chromium"), Some(Source::Chromium));
        assert_eq!(Source::parse("brave"), Some(Source::Brave));
        assert!(Source::parse("firefox").is_none());
        assert_eq!(Source::Chrome.keyring_application(), "chrome");
        assert_eq!(Source::Brave.keychain_service(), "Brave Safe Storage");
    }

    #[test]
    fn profiles_in_lists_default_first_and_only_importable_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Profile 2")).unwrap();
        test_db(&dir.path().join("Profile 2"), 0);
        std::fs::create_dir_all(dir.path().join("Default")).unwrap();
        test_db(&dir.path().join("Default"), 0);
        std::fs::create_dir_all(dir.path().join("Profile 1")).unwrap(); // no Cookies db
        std::fs::create_dir_all(dir.path().join("NotAProfile")).unwrap();
        assert_eq!(profiles_in(dir.path()), ["Default", "Profile 2"]);
    }
}
