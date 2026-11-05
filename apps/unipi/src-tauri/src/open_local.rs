//! "Open ▾" for files on the user's own PC (owner5 #23): reveal in the file
//! manager, open with the default app, or open in an editor found on this
//! PC. Desktop only; the phone/remote host keeps Download.

use unipi_proto::{AppErrorCode, AppErrorDto};

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenTarget {
    pub id: String,
    pub name: String,
}

/// Editors we look for on PATH: (binary, label).
const EDITORS: &[(&str, &str)] = &[
    ("code", "VS Code"),
    ("codium", "VSCodium"),
    ("cursor", "Cursor"),
    ("zed", "Zed"),
    ("subl", "Sublime Text"),
    ("kate", "Kate"),
    ("gedit", "Text Editor"),
];

#[cfg(desktop)]
fn search_path() -> std::ffi::OsString {
    let merged = crate::shell_path::merged_path();
    let mut parts: Vec<std::path::PathBuf> = std::env::split_paths(&merged).collect();
    if let Some(p) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(parts).unwrap_or_default()
}

#[cfg(desktop)]
fn find_in_path(bin: &str) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for dir in std::env::split_paths(&search_path()) {
            let candidate = dir.join(bin);
            if let Ok(meta) = std::fs::metadata(&candidate) {
                if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                    return Some(candidate);
                }
            }
        }
        None
    }
    #[cfg(windows)]
    {
        for dir in std::env::split_paths(&search_path()) {
            for ext in [".cmd", ".exe", ""] {
                let candidate = dir.join(format!("{bin}{ext}"));
                if std::fs::metadata(&candidate).map(|m| m.is_file()).unwrap_or(false) {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

/// Editors installed on this PC (the `id` is the binary name).
#[tauri::command]
pub fn local_open_targets() -> Vec<OpenTarget> {
    #[cfg(desktop)]
    {
        EDITORS
            .iter()
            .filter(|(bin, _)| find_in_path(bin).is_some())
            .map(|(bin, name)| OpenTarget {
                id: (*bin).to_string(),
                name: (*name).to_string(),
            })
            .collect()
    }
    #[cfg(not(desktop))]
    {
        let _ = EDITORS;
        Vec::new()
    }
}

#[cfg(not(desktop))]
fn unsupported() -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_UNSUPPORTED, "Not available on this device", false)
}

/// `how`: `reveal` | `default` | an editor id from `local_open_targets`.
#[tauri::command]
pub async fn local_open(path: String, how: String) -> Result<(), AppErrorDto> {
    #[cfg(desktop)]
    {
        use std::process::{Command, Stdio};
        let p = std::path::PathBuf::from(&path);
        if !p.is_absolute() || !p.exists() {
            return Err(AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "That file no longer exists", false));
        }
        let fail = |e: std::io::Error| {
            AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("can't open {path}: {e}"), false)
        };
        let spawn = |mut c: Command| {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                c.creation_flags(CREATE_NO_WINDOW);
            }
            c.env("PATH", search_path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(drop)
                .map_err(fail)
        };
        match how.as_str() {
            "default" => {
                #[cfg(target_os = "macos")]
                let mut c = Command::new("open");
                #[cfg(target_os = "windows")]
                let mut c = Command::new("explorer.exe");
                #[cfg(all(unix, not(target_os = "macos")))]
                let mut c = Command::new("xdg-open");
                c.arg(&p);
                spawn(c)
            }
            "reveal" => {
                #[cfg(target_os = "macos")]
                {
                    let mut c = Command::new("open");
                    c.arg("-R").arg(&p);
                    return spawn(c);
                }
                #[cfg(target_os = "windows")]
                {
                    let mut c = Command::new("explorer.exe");
                    c.arg(format!("/select,{}", p.display()));
                    return spawn(c);
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    if let (Ok(url), true) = (url::Url::from_file_path(&p), find_in_path("gdbus").is_some()) {
                        let ok = Command::new("gdbus")
                            .env("PATH", search_path())
                            .args([
                                "call", "--session", "--timeout", "3",
                                "--dest", "org.freedesktop.FileManager1",
                                "--object-path", "/org/freedesktop/FileManager1",
                                "--method", "org.freedesktop.FileManager1.ShowItems",
                            ])
                            .arg(format!("['{}']", url.as_str()))
                            .arg("")
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if ok {
                            return Ok(());
                        }
                    }
                    let mut c = Command::new("xdg-open");
                    c.arg(p.parent().unwrap_or(&p));
                    spawn(c)
                }
            }
            bin => {
                if !EDITORS.iter().any(|(b, _)| *b == bin) {
                    return Err(AppErrorDto::new(AppErrorCode::E_SCHEMA, "unknown editor", false));
                }
                let exe = find_in_path(bin).ok_or_else(|| {
                    AppErrorDto::new(AppErrorCode::E_NOT_FOUND, format!("{bin} isn't installed"), false)
                })?;
                let mut c = Command::new(exe);
                c.arg(&p);
                spawn(c)
            }
        }
    }
    #[cfg(not(desktop))]
    {
        let _ = (path, how);
        Err(unsupported())
    }
}

/// Native "Open folder…" dialog for the workspace picker (desktop, the
/// user's own PC). `None` = cancelled.
#[tauri::command]
pub async fn pick_folder(
    app: tauri::AppHandle,
    start: Option<String>,
) -> Result<Option<String>, AppErrorDto> {
    #[cfg(desktop)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Open folder");
            if let Some(dir) = start.filter(|d| std::path::Path::new(d).is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let pending = dialog.pick_folder();
            std::thread::spawn(move || {
                let picked =
                    tauri::async_runtime::block_on(pending).map(|h| h.path().to_path_buf());
                let _ = tx.send(picked);
            });
        })
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?;
        let picked = rx
            .await
            .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?;
        Ok(picked.map(|p| p.to_string_lossy().into_owned()))
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, start);
        Err(unsupported())
    }
}

/// One `zoxide` match: the folder and its frecency score.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct ZoxideHit {
    pub path: String,
    pub score: f64,
}

/// `zoxide query -ls` output → hits, best first (`  12.5 /home/u/proj`). Pure.
pub fn parse_zoxide(out: &str) -> Vec<ZoxideHit> {
    let mut hits: Vec<ZoxideHit> = out
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (score, path) = l.split_once(char::is_whitespace)?;
            let score = score.trim().parse::<f64>().ok()?;
            let path = path.trim();
            (!path.is_empty()).then(|| ZoxideHit { path: path.to_owned(), score })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits
}

/// "Quick navigate" (owner 2026-11-05): the folders `z <query>` would jump to on this PC,
/// best first (what `zoxide query -ls <words>` ranks). Empty when zoxide isn't installed.
#[tauri::command]
pub async fn local_zoxide(query: String) -> Result<Vec<ZoxideHit>, AppErrorDto> {
    #[cfg(all(desktop, unix))]
    {
        use std::process::{Command, Stdio};
        let Some(exe) = find_in_path("zoxide") else {
            return Err(AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "zoxide isn't installed on this PC", false));
        };
        let words: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
        let out = tokio::task::spawn_blocking(move || {
            Command::new(exe)
                .env("PATH", search_path())
                .args(["query", "-l", "-s", "--"])
                .args(&words)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
        })
        .await
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("zoxide: {e}"), false))?;
        let mut hits = parse_zoxide(&String::from_utf8_lossy(&out.stdout));
        hits.truncate(8);
        Ok(hits)
    }
    #[cfg(not(all(desktop, unix)))]
    {
        let _ = query;
        Err(AppErrorDto::new(AppErrorCode::E_UNSUPPORTED, "Not available on this device", false))
    }
}

#[cfg(test)]
mod zoxide_tests {
    use super::*;
    #[test]
    fn parses_scores_best_first() {
        let hits = parse_zoxide("   4.0 /home/u/a\n  12.5 /home/u/my proj\njunk\n");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "/home/u/my proj");
        assert!((hits[0].score - 12.5).abs() < 1e-9);
    }
}
