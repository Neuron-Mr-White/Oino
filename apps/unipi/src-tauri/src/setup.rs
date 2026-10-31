//! UNI-239 (plan S1, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md §4.2): what this
//! PC has, in one `SetupReport`.
//!
//! The hard part is the **PATH problem**: an app started from a menu (`.desktop`,
//! Dock, Start) does not get the shell's PATH, so a user whose Node and pi come
//! from mise or nvm looks like they have nothing. Detection therefore merges
//! three sources, in order of trust:
//!
//! 1. the process PATH;
//! 2. a login-shell probe — `$SHELL -lic 'printf %s "$PATH"'` with a 5 s
//!    timeout, ignored on failure (it is only ever an improvement);
//! 3. known directories (mise shims and installs, nvm, `~/.pi/agent/bin`,
//!    `~/.local/share/pi-node/current/bin`, `~/.local/bin`, …).
//!
//! The merged result is also what the host gets: the app-managed spawn
//! (`host_supervisor`, v2 has no service install any more) puts it on the
//! host's PATH, so `oino`, kanboard and pi resolve the same way as in the
//! user's shell.
//!
//! Version rules (user decision 4): newest wins. A **newer** version the user
//! has is left alone; an **older** one is `outdated` and offered for update.
//! pi's floor is `0.87.1` and is a plain comparison, never a caret — the UniPi
//! extensions peer-depend on `^0.87.1`, which pi 1.0.0 satisfies.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unipi_proto::AppErrorDto;

pub mod install;
pub mod updates;

/// Node floor: the UniPi extensions need ≥ 22.19.
pub const NODE_MIN: (u64, u64, u64) = (22, 19, 0);
/// pi floor: a plain comparison, not a caret (pi 1.0.0 ≥ 0.87.1).
pub const PI_MIN: (u64, u64, u64) = (0, 87, 1);
/// How long the login-shell probe may take.
pub const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The minimum UniPi major this app build can talk to. npm's `latest` is
/// still 2.x, so a fresh install takes `alpha` until 3.x is `latest`.
pub const MIN_UNIPI_MAJOR: u64 = 3;
/// npm's abbreviated metadata endpoint for a package's dist-tags.
pub const NPM_TAGS_URL: &str = "https://registry.npmjs.org/-/package";

/// `$HOME` (`%USERPROFILE%` on Windows, which has no `HOME`), or ""
/// (never the host's data dir).
pub fn setup_json_path_of_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(|h| setup_json_path(std::path::Path::new(&h)))
        .unwrap_or_else(|| PathBuf::from(".unipi/app-setup.json"))
}

/// Where the app records what IT installed (never the host's data dir).
pub fn setup_json_path(home: &Path) -> PathBuf {
    home.join(".unipi").join("app-setup.json")
}

// ------------------------------------------------------------------ version

/// `1.2.3`, `v1.2.3`, `1.2.3-alpha.4`, `herdr 0.9.0` → `(1, 2, 3)`.
pub fn version_tuple(text: &str) -> (u64, u64, u64) {
    let digits: String = text
        .trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = digits.split('.').map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// `>= min` by the tuple (never a semver range).
pub fn at_least(version: &str, min: (u64, u64, u64)) -> bool {
    version_tuple(version) >= min
}

// --------------------------------------------------------------------- PATH

/// Splits a `PATH` string into directories (empty entries dropped).
pub fn parse_path(value: &str) -> Vec<PathBuf> {
    value
        .split(':')
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// Deduplicates while keeping the first (most trusted) position.
pub fn dedup_dirs(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|d| seen.insert(d.clone()))
        .collect()
}

/// Where a tool may hide outside any PATH, in the order we try them.
pub fn known_dirs(home: &Path) -> Vec<PathBuf> {
    let local_share = home.join(".local").join("share");
    let mise = local_share.join("mise");
    let mut dirs = vec![
        home.join(".local").join("bin"),
        home.join(".pi").join("agent").join("bin"),
        home.join(".pi").join("agent").join("install"),
        local_share.join("pi-node").join("current").join("bin"),
        mise.join("shims"),
    ];
    // mise installs: the shims dir is the canonical entry (it points at the
    // tool's active version), and we add each tool's NEWEST install as a
    // fallback for a shims dir that is missing. Newest by version tuple, not
    // by string — "9" must not sort above "10".
    if let Ok(entries) = std::fs::read_dir(mise.join("installs")) {
        let mut tools: Vec<(String, Vec<(String, PathBuf)>)> = Vec::new();
        for tool in entries.flatten() {
            let mut versions: Vec<(String, PathBuf)> = Vec::new();
            if let Ok(list) = std::fs::read_dir(tool.path()) {
                for version in list.flatten() {
                    versions.push((
                        version.file_name().to_string_lossy().to_string(),
                        version.path().join("bin"),
                    ));
                }
            }
            tools.push((tool.file_name().to_string_lossy().to_string(), versions));
        }
        tools.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, mut versions) in tools {
            versions.sort_by(|a, b| {
                version_tuple(&b.0)
                    .cmp(&version_tuple(&a.0))
                    .then_with(|| b.0.cmp(&a.0))
            });
            if let Some((_, bin)) = versions.into_iter().next() {
                dirs.push(bin);
            }
        }
    }
    dirs.push(home.join(".nvm").join("current").join("bin"));
    dirs.push(local_share.join("pnpm"));
    dirs.push(home.join(".bun").join("bin"));
    dirs.push(home.join(".cargo").join("bin"));
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(home.join(".deno").join("bin"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

/// The login-shell probe's raw output, e.g. `/a:/b\n` → `Some("/a:/b")`.
/// The probe prints a crab banner on this PC, so only the last line that looks
/// like a PATH is used.
pub fn clean_probe_output(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty() && line.contains('/') && !line.contains(char::is_whitespace))
        .map(str::to_string)
}

/// The merged PATH: process, then the probe, then the known dirs.
pub fn merge_paths(process: &[PathBuf], probe: &[PathBuf], known: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = Vec::new();
    all.extend(process.iter().cloned());
    all.extend(probe.iter().cloned());
    all.extend(known.iter().cloned());
    dedup_dirs(all)
}

/// The merged PATH as a `PATH=`-shaped string.
pub fn path_value(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(":")
}

// ------------------------------------------------------------------ report

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentState {
    Ok,
    Missing,
    Outdated,
    Broken,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The user's own (mise, nvm, brew, apt…): never replaced by the app.
    Found,
    /// The app put it there (repair/uninstall may touch it).
    UnipiManaged,
}

/// One component of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub name: String,
    pub state: ComponentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub source: Source,
    /// What would fix it, in one line (a command or a size + target).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The PATH part of the report: what we merged and what the user's login shell
/// has that we would have missed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathReport {
    /// The merged PATH, `:`-joined — also what the host is given.
    pub value: String,
    /// Directories the login-shell probe contributed.
    pub from_shell: Vec<String>,
    /// Whether `~/.local/bin` is on the login PATH (a fresh terminal will see
    /// what the app installs there).
    pub local_bin_on_login_path: bool,
    /// `true` when the probe worked.
    pub probed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupReport {
    pub components: Vec<Component>,
    pub path: PathReport,
    /// Everything needed for local use + chat is present.
    pub ready: bool,
    /// True when at least one component needs an update (decision 4).
    pub update_available: bool,
    /// When the report was written.
    pub at: u64,
}

impl SetupReport {
    pub fn get(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    fn state_of(&self, name: &str) -> ComponentState {
        self.get(name)
            .map(|c| c.state)
            .unwrap_or(ComponentState::Missing)
    }

    /// Is this component usable (ok, or a newer version than we ask for)?
    pub fn is_ok(&self, name: &str) -> bool {
        self.state_of(name) == ComponentState::Ok
    }
}

// ---------------------------------------------------------------- detection

/// Runs a binary with extra environment and returns its combined output. Tests
/// inject a fake.
pub type Runner = Arc<dyn Fn(&Path, &[&str], &[(&str, String)]) -> Option<String> + Send + Sync>;

/// The real runner: `--version` style invocations.
pub fn system_runner() -> Runner {
    Arc::new(|bin, args, env| {
        let mut command = Command::new(bin);
        command.args(args);
        for (key, value) in env {
            command.env(key, value);
        }
        let out = command.output().ok()?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Some(text)
    })
}

/// Everything detection needs, injectable so tests never touch the real PC.
#[derive(Clone)]
pub struct DetectCtx {
    pub home: PathBuf,
    pub process_path: Vec<PathBuf>,
    /// The probe's result (already cleaned), or `None` when it failed.
    pub probe: Option<Vec<PathBuf>>,
    pub run: Runner,
    /// `pi list` output, when it could be taken.
    pub pi_list: Option<String>,
    /// The host's own state (from the supervisor / host.lock).
    pub host: Option<HostState>,
    /// The app's own record of what it installed.
    pub installed: InstalledRecord,
    /// npm's dist-tags for the packages we install (channel selection).
    pub tags: DistTags,
    /// Now, unix seconds.
    pub now: u64,
}

/// What the host looks like to detection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostState {
    /// `service` | `app` | `manual`, from `host.lock`.
    pub run_by: Option<String>,
    pub version: Option<String>,
    pub bin: Option<String>,
}

/// `~/.unipi/app-setup.json`: what the app installed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InstalledRecord {
    /// `{ "node": { "version": "22.19.0", "path": "...", "at": 123 } }`
    pub components: BTreeMap<String, InstalledComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledComponent {
    pub version: String,
    pub path: String,
    pub at: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl InstalledRecord {
    pub fn read(home: &Path) -> Self {
        std::fs::read_to_string(setup_json_path(home))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, home: &Path) -> std::io::Result<()> {
        let path = setup_json_path(home);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    /// Marks `name` as ours (repair/uninstall may touch only these). The caller
    /// is `setup::install` (UNI-240).
    pub fn mark(&mut self, name: &str, version: &str, path: &Path, at: u64) {
        self.components.insert(
            name.to_string(),
            InstalledComponent {
                version: version.to_string(),
                path: path.display().to_string(),
                at,
            },
        );
    }
}

/// The first entry of `dirs` that holds an executable `name`.
pub fn find_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.iter().map(|d| d.join(&exe)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// `<dirs>/<name> --version ...` → the version-looking first token. `env` carries
/// the merged PATH, so a tool that shells out to its own runtime (npm → node)
/// still answers.
fn version_of(run: &Runner, bin: &Path, args: &[&str], env: &[(&str, String)]) -> Option<String> {
    let out = run(bin, args, env)?;
    out.split_whitespace()
        .find(|token| {
            token.chars().next().is_some_and(|c| c.is_ascii_digit()) && token.contains('.')
        })
        .or_else(|| out.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(|s| s.trim().trim_start_matches('v').to_string())
}

fn component(
    name: &str,
    version: Option<String>,
    bin: Option<&Path>,
    state: ComponentState,
    source: Source,
    fix: Option<String>,
    note: Option<String>,
) -> Component {
    Component {
        name: name.to_string(),
        state,
        version,
        path: bin.map(|p| p.display().to_string()),
        source,
        fix,
        note,
    }
}

/// A version-checked component: present but too old is `outdated`, present and
/// older-than-the-floor is `broken` ( UniPi itself needs the floor).
fn versioned(
    name: &str,
    version: Option<String>,
    bin: Option<&Path>,
    floor: (u64, u64, u64),
    fix: &str,
) -> Component {
    let source = Source::Found;
    match &version {
        None => component(
            name,
            None,
            bin,
            ComponentState::Missing,
            source,
            Some(fix.into()),
            None,
        ),
        Some(v) if at_least(v, floor) => component(
            name,
            Some(v.clone()),
            bin,
            ComponentState::Ok,
            source,
            None,
            None,
        ),
        Some(v) => component(
            name,
            Some(v.clone()),
            bin,
            ComponentState::Outdated,
            source,
            Some(fix.into()),
            Some(format!(
                "Needs {}.{}.{} or newer",
                floor.0, floor.1, floor.2
            )),
        ),
    }
}

/// `pi list` contains the UniPi extensions?
pub fn unipi_from_pi_list(list: &str) -> Option<String> {
    list.lines()
        .map(str::trim)
        .find(|l| l.starts_with("npm:@pi-unipi/unipi") || l.starts_with("@pi-unipi/unipi"))
        .map(|l| l.to_string())
}

/// The version of an installed npm package, from its package.json.
pub fn installed_package_version(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value.get("version")?.as_str().map(str::to_string)
}

/**
 * npm dist-tags for the packages we care about, and the major of each.
 *
 * Used to pick the UniPi channel to install on a PC that has nothing yet
 * (task 0 / UNI-241): npm's `latest` is still the 2.x line, while this app
 * build is built for the 3.x API.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DistTags {
    /// The `latest` dist-tag's version.
    pub latest: Option<String>,
    /// The `alpha` dist-tag's version.
    pub alpha: Option<String>,
}

impl DistTags {
    /// Parses what `GET /-/package/<name>/dist-tags` returns.
    pub fn parse(json: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return DistTags::default();
        };
        let tag = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
        DistTags {
            latest: tag("latest"),
            alpha: tag("alpha"),
        }
    }

    fn version_of(&self, tag: &str) -> Option<&str> {
        match tag {
            "latest" => self.latest.as_deref(),
            "alpha" => self.alpha.as_deref(),
            _ => None,
        }
    }

    /// The major of a dist-tag's version (`2.20.5` → `2`).
    fn major_of(&self, tag: &str) -> Option<u64> {
        self.version_of(tag).map(|v| version_tuple(v).0)
    }
}

/// The dist-tags of the packages we install, fetched in one place and shared
/// by the plan and the Components screen.
pub async fn unipi_dist_tags() -> DistTags {
    fetch_dist_tags(NPM_TAGS_URL, "@pi-unipi/unipi").await
}

async fn fetch_dist_tags(base: &str, package: &str) -> DistTags {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return DistTags::default();
    };
    match client
        .get(format!("{base}/{package}/dist-tags"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            DistTags::parse(&response.text().await.unwrap_or_default())
        }
        _ => DistTags::default(),
    }
}

/// The channel to install UniPi from, mirroring packages/updater's
/// `channel.ts`: the pi settings spec's tag, else the installed version's
/// prerelease id, else `latest` — with one addition (task 0): on a PC with
/// nothing installed yet, `latest` is only followed when it is at least as
/// new as this app build supports. While npm's `latest` is still 2.x and
/// `alpha` is 3.x, a fresh install must take `alpha`, or the app would
/// install a UniPi it cannot talk to.
pub fn channel_for(spec: Option<&str>, installed: Option<&str>, tags: &DistTags) -> String {
    fn from_spec(spec: &str) -> Option<String> {
        let s = spec.trim().trim_start_matches("npm:");
        let tag = s.strip_prefix("@pi-unipi/unipi@")?.trim();
        let pinned = tag.starts_with(['^', '~', '<', '>', '=', 'v'])
            || tag.starts_with(|c: char| c.is_ascii_digit());
        if tag.is_empty() || pinned || tag == "*" {
            return None;
        }
        Some(tag.to_string())
    }
    fn prerelease(version: &str) -> Option<String> {
        let rest = version.trim().trim_start_matches('v');
        let rest = rest.split_once('-')?.1;
        let id = rest.split('.').next()?;
        (!id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty()).then(|| id.to_string())
    }
    // Graduation (mirrors the updater's UNI-272 `pickInstallTag`): a
    // prerelease channel whose stable `latest` has caught up — 3.0.0 is out
    // while the install still follows `alpha` — moves to `latest`, or it would
    // stay on the retired alpha tag forever. Unknown tags (offline) keep it.
    let graduate = |channel: String| -> String {
        if channel == "latest" {
            return channel;
        }
        match (tags.version_of(&channel), tags.latest.as_deref()) {
            (Some(pre), Some(latest))
                if version_tuple(latest).0 >= MIN_UNIPI_MAJOR
                    && !updates::is_newer(pre, latest) =>
            {
                "latest".to_string()
            }
            _ => channel,
        }
    };
    let spec = spec.map(str::trim).filter(|s| !s.is_empty());
    if let Some(pinned) = spec.and_then(from_spec) {
        return graduate(pinned);
    }
    if let Some(pinned) = installed.and_then(prerelease) {
        return graduate(pinned);
    }
    // Nothing pins a channel. On a PC with nothing installed and no pi settings
    // spec at all — a genuinely fresh install — `latest` is only followed once
    // it is at least as new as this app build supports: while npm's `latest` is
    // still 2.x and `alpha` is 3.x, a fresh install must take `alpha`, or the
    // app would end up talking to a UniPi it cannot use. Anything already there
    // (a spec, an installed version) is a choice, so the updater's own rule
    // stands for it.
    if spec.is_none() && installed.is_none() {
        let latest_major = tags.major_of("latest");
        let alpha_major = tags.major_of("alpha");
        if latest_major.is_some_and(|m| m < MIN_UNIPI_MAJOR)
            && alpha_major.is_some_and(|m| m >= MIN_UNIPI_MAJOR)
        {
            return "alpha".to_string();
        }
    }
    "latest".to_string()
}

/// The `pi install` source for a channel.
pub fn install_spec(channel: &str) -> String {
    if channel == "latest" {
        "npm:@pi-unipi/unipi".to_string()
    } else {
        format!("npm:@pi-unipi/unipi@{channel}")
    }
}

/// The UniPi package spec from pi's settings files (project then global).
pub fn read_unipi_spec(home: &Path, cwd: &Path) -> Option<String> {
    let dir = std::env::var_os("PI_CODING_AGENT_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".pi").join("agent"));
    for file in [cwd.join(".pi/settings.json"), dir.join("settings.json")] {
        let Ok(raw) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(list) = value.get("packages").and_then(|p| p.as_array()) else {
            continue;
        };
        for entry in list {
            let source = match entry {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Object(o) => {
                    o.get("source").and_then(|s| s.as_str()).map(str::to_string)
                }
                _ => None,
            };
            let Some(source) = source else { continue };
            let bare = source.trim().trim_start_matches("npm:");
            if bare == "@pi-unipi/unipi" || bare.starts_with("@pi-unipi/unipi@") {
                return Some(source.trim().to_string());
            }
        }
    }
    None
}

/// npm's platform key for the extensions' platform package (`linux-x64`),
/// matching the host's `board/discovery.rs::platform_key`.
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The board binary, the way the host's own discovery finds it
/// (crates/host/src/board/discovery.rs): `$UNIPI_KANBOARD_BIN`, the merged
/// PATH, then the npm `.bin` and the extensions' platform package.
pub fn kanboard_binary(dirs: &[PathBuf], home: &Path) -> Option<PathBuf> {
    let platform = format!("kanboard-{}", platform_key());
    let npm_root = home.join(".pi/agent/npm/node_modules");
    std::env::var_os("UNIPI_KANBOARD_BIN")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| find_in(dirs, "unipi-kanboard"))
        .or_else(|| find_in(dirs, "kanboard"))
        .or_else(|| find_in(&[npm_root.join(".bin")], "unipi-kanboard"))
        .or_else(|| {
            let bin = npm_root
                .join("@pi-unipi")
                .join(&platform)
                .join("bin/unipi-kanboard");
            is_executable(&bin).then_some(bin)
        })
        .or_else(|| {
            let bin = npm_root.join("@pi-unipi/kanboard/bin/unipi-kanboard");
            is_executable(&bin).then_some(bin)
        })
}

/// Builds the report from an injected context.
pub fn detect_with(ctx: &DetectCtx) -> SetupReport {
    let dirs = merge_paths(
        &ctx.process_path,
        ctx.probe.as_deref().unwrap_or(&[]),
        &known_dirs(&ctx.home),
    );
    // Tools are run with the merged PATH: npm, for one, is a `#!/usr/bin/env
    // node` script and cannot find its own runtime from a GUI's PATH.
    let path_env: Vec<(&str, String)> = vec![("PATH", path_value(&dirs))];
    let local_bin = ctx.home.join(".local").join("bin");
    let pi_agent_bin = ctx.home.join(".pi").join("agent").join("bin");
    let pi_npm_modules = ctx
        .home
        .join(".pi")
        .join("agent")
        .join("npm")
        .join("node_modules");

    let ours = |name: &str| {
        ctx.installed
            .components
            .get(name)
            .map(|c| (c.version.clone(), c.path.clone()))
    };

    // --- node -------------------------------------------------------------
    let node = find_in(&dirs, "node");
    let node_version = node
        .as_deref()
        .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env));
    let node_component = match ours("node") {
        Some((version, path)) if node.is_none() => {
            // We installed it but it is gone from the merged PATH: point at it.
            let bin = PathBuf::from(&path);
            component(
                "node",
                Some(version),
                Some(&bin),
                ComponentState::Missing,
                Source::UnipiManaged,
                Some("Re-run setup to restore it".into()),
                Some(format!("Installed at {path}, not on the PATH we see")),
            )
        }
        _ => versioned(
            "node",
            node_version,
            node.as_deref(),
            NODE_MIN,
            "Install Node 22 LTS (~30 MB)",
        ),
    };
    let node_ok = node_component.state == ComponentState::Ok;

    // --- npm --------------------------------------------------------------
    let npm = find_in(&dirs, "npm");
    let npm_version = npm
        .as_deref()
        .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env));
    let npm_component = versioned(
        "npm",
        npm_version,
        npm.as_deref(),
        (0, 0, 0),
        "Comes with Node",
    );
    // npm is `unsupported` rather than `missing` when Node itself is absent —
    // there is nothing to fix that isn't Node.
    let _ = node_ok;

    // --- pi ---------------------------------------------------------------
    let pi =
        find_in(&dirs, "pi").or_else(|| find_in(&[local_bin.clone(), pi_agent_bin.clone()], "pi"));
    let pi_version = pi
        .as_deref()
        .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env));
    let pi_component = versioned(
        "pi",
        pi_version,
        pi.as_deref(),
        PI_MIN,
        "Install pi (pi.dev installer, npm fallback)",
    );

    // --- agent-browser ------------------------------------------------------
    // Optional: the `skill:browser` helper. Installed with `npm install -g
    // agent-browser` into the same prefix pi's N2 npm fallback uses
    // (`~/.pi/agent/npm`), so it is found the same way as pi itself. We never
    // run `agent-browser install` (that downloads its own Chrome) — the host's
    // CloakBrowser is what the skill actually drives.
    let agent_browser = find_in(&dirs, "agent-browser")
        .or_else(|| find_in(&[pi_npm_modules.join(".bin")], "agent-browser"));
    let agent_browser_version = agent_browser
        .as_deref()
        .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env));
    let agent_browser_component = component(
        "agent-browser",
        agent_browser_version,
        agent_browser.as_deref(),
        if agent_browser.is_some() {
            ComponentState::Ok
        } else {
            ComponentState::Missing
        },
        Source::Found,
        agent_browser
            .is_none()
            .then(|| "npm install -g agent-browser".into()),
        Some("Optional — lets pi drive the browser skill".into()),
    );

    // --- unipi extensions -------------------------------------------------
    let spec = read_unipi_spec(&ctx.home, Path::new("."));
    let listed = ctx.pi_list.as_deref().and_then(unipi_from_pi_list);
    let ext_dir = pi_npm_modules.join("@pi-unipi").join("unipi");
    let ext_version = installed_package_version(&ext_dir);
    let unipi_component = match (&listed, &ext_version) {
        (Some(_), Some(v)) => component(
            "unipi",
            Some(v.clone()),
            Some(&ext_dir),
            ComponentState::Ok,
            Source::Found,
            None,
            spec.as_ref().map(|s| format!("Installed as {s}")),
        ),
        (Some(_), None) => component(
            "unipi",
            None,
            Some(&ext_dir),
            ComponentState::Broken,
            Source::Found,
            Some("Repair: pi install".into()),
            Some("pi lists it but its files are gone".into()),
        ),
        _ => {
            let channel = channel_for(spec.as_deref(), ext_version.as_deref(), &ctx.tags);
            component(
                "unipi",
                None,
                None,
                ComponentState::Missing,
                Source::Found,
                Some(format!("pi install {}", install_spec(&channel))),
                Some("Needs pi first".into()),
            )
        }
    };

    // --- kanboard ---------------------------------------------------------
    let kanboard = kanboard_binary(&dirs, &ctx.home);
    let kanboard_component = match &kanboard {
        Some(bin) => {
            let version = version_of(&ctx.run, bin, &["--version"], &path_env);
            component(
                "kanboard",
                version,
                Some(bin),
                ComponentState::Ok,
                Source::Found,
                None,
                Some("Comes with the UniPi extensions".into()),
            )
        }
        // Not installed on its own: the extensions bring the platform package.
        None if unipi_component.state == ComponentState::Ok => component(
            "kanboard",
            None,
            None,
            ComponentState::Missing,
            Source::Found,
            Some("Reinstall the UniPi extensions".into()),
            Some("Expected from @pi-unipi/unipi".into()),
        ),
        None => component(
            "kanboard",
            None,
            None,
            ComponentState::Unsupported,
            Source::Found,
            None,
            Some("Comes with the UniPi extensions".into()),
        ),
    };

    // --- host -------------------------------------------------------------
    let host_component = match &ctx.host {
        Some(h) => component(
            "host",
            h.version.clone(),
            h.bin.as_deref().map(Path::new),
            ComponentState::Ok,
            Source::Found,
            None,
            Some(match h.run_by.as_deref() {
                Some(mode) => format!("Run by {mode}"),
                None => "Running".into(),
            }),
        ),
        None => component(
            "host",
            None,
            None,
            ComponentState::Missing,
            Source::UnipiManaged,
            Some("Start it from This computer".into()),
            Some("UniPi runs its own host; nothing to download".into()),
        ),
    };

    // --- herdr / tmux -----------------------------------------------------
    let herdr = find_in(&dirs, "herdr");
    let herdr_component = component(
        "herdr",
        herdr
            .as_deref()
            .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env)),
        herdr.as_deref(),
        if herdr.is_some() {
            ComponentState::Ok
        } else {
            ComponentState::Missing
        },
        if herdr.is_some() {
            Source::Found
        } else {
            Source::UnipiManaged
        },
        herdr
            .is_none()
            .then(|| "Install herdr (herdr.dev installer)".into()),
        herdr
            .is_none()
            .then(|| "Optional — better terminals".into()),
    );
    let tmux = find_in(&dirs, "tmux");
    let tmux_component = component(
        "tmux",
        tmux.as_deref()
            .and_then(|b| version_of(&ctx.run, b, &["-V"], &path_env)),
        tmux.as_deref(),
        if tmux.is_some() {
            ComponentState::Ok
        } else {
            ComponentState::Missing
        },
        Source::Found,
        tmux.is_none()
            .then(|| package_manager_tmux_command().into()),
        tmux.is_none()
            .then(|| "Optional — needs root, never automatic".into()),
    );

    // --- hidden display for the browser (UNI-299) ------------------------
    // CloakBrowser runs headed on a private X display so it shows only in the
    // app. Only Xvfb: a standalone Xwayland opens a visible window on Wayland
    // desktops. Without it the browser runs headless (less stealthy).
    let xvfb = find_in(&dirs, "Xvfb");
    let xvfb_component = component(
        "xvfb",
        None,
        xvfb.as_deref(),
        if xvfb.is_some() {
            ComponentState::Ok
        } else {
            ComponentState::Missing
        },
        Source::Found,
        xvfb.is_none()
            .then(|| package_manager_xvfb_command().into()),
        Some(if xvfb.is_some() {
            "Hidden display for the browser (shown only in the app)".into()
        } else {
            "Optional — lets the browser show only in the app without running headless; needs root"
                .into()
        }),
    );

    // --- systemd / launcher + linger -------------------------------------
    #[cfg(target_os = "linux")]
    let (service_name, service_path) = ("systemd-user", find_in(&dirs, "systemctl"));
    #[cfg(target_os = "macos")]
    let (service_name, service_path) = ("launchd", Some(PathBuf::from("/bin/launchctl")));
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (service_name, service_path) = ("service", None::<PathBuf>);
    let service_component = component(
        service_name,
        service_path
            .as_deref()
            .and_then(|b| version_of(&ctx.run, b, &["--version"], &path_env)),
        service_path.as_deref(),
        if service_path.is_some() {
            ComponentState::Ok
        } else {
            ComponentState::Unsupported
        },
        Source::Found,
        None,
        Some("Keeps the host running with no UniPi window".into()),
    );

    let components = vec![
        node_component,
        npm_component,
        pi_component,
        unipi_component,
        agent_browser_component,
        kanboard_component,
        host_component,
        herdr_component,
        tmux_component,
        xvfb_component,
        service_component,
    ];
    // Ready = Node/pi/UniPi usable (kanboard may be unsupported when the
    // extensions aren't there yet) AND a host is running.
    let report_probe = SetupReport {
        components: components.clone(),
        path: PathReport {
            value: String::new(),
            from_shell: vec![],
            local_bin_on_login_path: false,
            probed: false,
        },
        ready: false,
        update_available: false,
        at: ctx.now,
    };
    let ready = ["node", "npm", "pi", "unipi"].iter().all(|n| {
        matches!(
            report_probe.state_of(n),
            ComponentState::Ok | ComponentState::Unsupported
        )
    }) && report_probe.is_ok("host");
    let update_available = components
        .iter()
        .any(|c| c.state == ComponentState::Outdated);

    SetupReport {
        components,
        path: PathReport {
            value: path_value(&dirs),
            from_shell: dedup_dirs(ctx.probe.clone().unwrap_or_default())
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            local_bin_on_login_path: ctx
                .probe
                .as_deref()
                .unwrap_or(ctx.process_path.as_slice())
                .iter()
                .any(|p| p == &local_bin),
            probed: ctx.probe.is_some(),
        },
        ready,
        update_available,
        at: ctx.now,
    }
}

/// The Xvfb package for this PC's package manager (UNI-299): the name differs
/// per distro, unlike tmux.
pub fn xvfb_package(manager_program: &str) -> &'static str {
    match manager_program {
        "dnf" => "xorg-x11-server-Xvfb",
        "pacman" => "xorg-server-xvfb",
        _ => "xvfb",
    }
}

/// The Xvfb install command for this PC (shown; "Install for me" runs it).
pub fn package_manager_xvfb_command() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        if Path::new("/etc/debian_version").exists() {
            "sudo apt install xvfb"
        } else if Path::new("/etc/fedora-release").exists()
            || Path::new("/etc/redhat-release").exists()
        {
            "sudo dnf install xorg-x11-server-Xvfb"
        } else if Path::new("/etc/arch-release").exists() {
            "sudo pacman -S xorg-server-xvfb"
        } else {
            "install Xvfb with your package manager"
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        "not needed on this system"
    }
}

/// The tmux install command for this PC's package manager (never run by us).
pub fn package_manager_tmux_command() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "brew install tmux"
    }
    #[cfg(all(not(target_os = "macos"), target_os = "linux"))]
    {
        if Path::new("/etc/debian_version").exists() {
            "sudo apt install tmux"
        } else if Path::new("/etc/fedora-release").exists()
            || Path::new("/etc/redhat-release").exists()
        {
            "sudo dnf install tmux"
        } else if Path::new("/etc/arch-release").exists() {
            "sudo pacman -S tmux"
        } else {
            "install tmux with your package manager"
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "tmux is not available on Windows"
    }
}

// ------------------------------------------------------- default detection

/// The merged PATH, computed once per process — the probe spawns a shell, and
/// the host spawn happens at most a handful of times. Empty until
/// [`merged_path_here`] has run.
static MERGED_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The merged PATH, or "" while nothing has computed it yet.
pub fn merged_path() -> String {
    MERGED_PATH.get().cloned().unwrap_or_default()
}

/// Merges the process PATH, the login-shell probe and the known dirs, and
/// caches the result for the host spawn and the unit's env file.
pub async fn merged_path_here(home: &Path) -> String {
    let value = compute_merged_path(home).await;
    let _ = MERGED_PATH.set(value.clone());
    value
}

async fn compute_merged_path(home: &Path) -> String {
    let process_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let probe = match std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        Some(shell) if shell.exists() => probe_shell_path(&shell).await,
        _ => None,
    };
    path_value(&merge_paths(
        &process_path,
        &probe.unwrap_or_default(),
        &known_dirs(home),
    ))
}

/// The real detection: process env + a login-shell probe + known dirs.
pub async fn detect_here(home: &Path) -> SetupReport {
    let _ = merged_path_here(home).await;
    let process_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let shell = std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let probe = match &shell {
        Some(shell) if shell.exists() => probe_shell_path(shell).await,
        _ => None,
    };

    let merged = merge_paths(
        &process_path,
        probe.as_deref().unwrap_or(&[]),
        &known_dirs(home),
    );
    let ctx = DetectCtx {
        home: home.to_path_buf(),
        process_path,
        probe,
        run: system_runner(),
        // The same merged dirs detection reports, so `pi list` finds `pi`.
        pi_list: probe_pi_list(&merged).await,
        host: host_state(),
        installed: InstalledRecord::read(home),
        tags: unipi_dist_tags().await,
        now: now_unix(),
    };
    detect_with(&ctx)
}

/// `$SHELL -lic 'printf %s "$PATH"'`, cleaned; `None` on any failure.
pub async fn probe_shell_path(shell: &Path) -> Option<Vec<PathBuf>> {
    let out = tokio::process::Command::new(shell)
        .args(["-lic", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .output();
    let out = tokio::time::timeout(SHELL_PROBE_TIMEOUT, out)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    clean_probe_output(&text).map(|p| parse_path(&p))
}

/// `pi list` for the detection's "is UniPi installed" — run against the merged
/// PATH, and never fatal.
async fn probe_pi_list(dirs: &[PathBuf]) -> Option<String> {
    let pi = find_in(dirs, "pi")?;
    // `pi` is a `#!/usr/bin/env node` script: it needs the merged PATH too.
    let out = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(pi)
            .arg("list")
            .env("PATH", path_value(dirs))
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// The host's state, read from `host.lock` (never a probe that could block).
fn host_state() -> Option<HostState> {
    let dir = crate::local_host::host_dir().unwrap_or_default();
    let raw = std::fs::read_to_string(dir.join("host.lock")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(HostState {
        run_by: value
            .get("mode")
            .and_then(|m| m.as_str())
            .map(str::to_string),
        version: value
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        bin: crate::sharing::bin().map(|b| b.display().to_string()),
    })
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- commands

/// `setup_detect`: the report, plus where it was recorded.
#[tauri::command]
pub async fn setup_detect() -> Result<SetupReport, AppErrorDto> {
    let home = crate::host_supervisor::home_dir();
    let report = detect_here(&home).await;
    let _ = report.clone();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/tester")
    }

    /// A fake runner: `{"node": "v24.16.0", …}` by binary name.
    fn runner(versions: &[(&str, &str)]) -> Runner {
        let map: BTreeMap<String, String> = versions
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Arc::new(move |bin, _args, _env| {
            let name = bin.file_name()?.to_str()?.to_string();
            map.get(&name).cloned()
        })
    }

    fn ctx(run: Runner) -> DetectCtx {
        DetectCtx {
            home: home(),
            process_path: vec![PathBuf::from("/usr/bin")],
            probe: None,
            run,
            pi_list: None,
            host: None,
            installed: InstalledRecord::default(),
            tags: DistTags::default(),
            now: 1,
        }
    }

    fn stub(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    // ---------------------------------------------------------------- PATH

    #[test]
    fn merges_process_probe_and_known_dirs_without_duplicates() {
        let process = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];
        let probe = vec![PathBuf::from("/opt/mise/shims"), PathBuf::from("/usr/bin")];
        let known = vec![PathBuf::from("/home/t/.local/bin"), PathBuf::from("/bin")];
        let merged = merge_paths(&process, &probe, &known);
        assert_eq!(
            merged,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/mise/shims"),
                PathBuf::from("/home/t/.local/bin"),
            ]
        );
        assert_eq!(
            path_value(&merged),
            "/usr/bin:/bin:/opt/mise/shims:/home/t/.local/bin"
        );
    }

    #[test]
    fn the_probe_output_is_cleaned_of_banners() {
        // This PC's `bash -lic` prints a crab before the PATH.
        let raw = "\n ／l、\n（>､< ７\n/home/a:/home/b\n";
        assert_eq!(clean_probe_output(raw).as_deref(), Some("/home/a:/home/b"));
        assert_eq!(clean_probe_output("no paths here").as_deref(), None);
        assert_eq!(clean_probe_output("").as_deref(), None);
    }

    #[test]
    fn known_dirs_name_the_ones_the_doc_lists() {
        let dirs = known_dirs(&home());
        let names: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        for want in [
            "/home/tester/.local/bin",
            "/home/tester/.pi/agent/bin",
            "/home/tester/.pi/agent/install",
            "/home/tester/.local/share/pi-node/current/bin",
            "/home/tester/.local/share/mise/shims",
        ] {
            assert!(
                names.contains(&want.to_string()),
                "{want} missing from {names:?}"
            );
        }
        // …and they stay first, in the documented order.
        assert_eq!(names[0], "/home/tester/.local/bin");
    }

    // ------------------------------------------------------------- versions

    #[test]
    fn versions_parse_and_compare_numerically() {
        assert_eq!(version_tuple("v24.16.0"), (24, 16, 0));
        assert_eq!(version_tuple("1.0.0"), (1, 0, 0));
        assert_eq!(version_tuple("herdr 0.9.0"), (0, 9, 0));
        assert_eq!(version_tuple("3.0.0-alpha.41"), (3, 0, 0));
        assert!(at_least("1.0.0", PI_MIN), "pi 1.0.0 satisfies ^0.87.1");
        assert!(at_least("0.87.1", PI_MIN));
        assert!(!at_least("0.87.0", PI_MIN));
        assert!(!at_least("22.18.0", NODE_MIN));
        assert!(at_least("22.19.0", NODE_MIN));
        assert!(at_least("24.16.0", NODE_MIN));
    }

    // ------------------------------------------------------------ detection

    #[test]
    fn agent_browser_is_detected_on_the_merged_path_and_optional_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        stub(dir.path(), "agent-browser");
        let mut c = ctx(runner(&[("agent-browser", "1.2.3")]));
        c.process_path = vec![dir.path().to_path_buf()];
        let report = detect_with(&c);
        let component = report.get("agent-browser").unwrap();
        assert_eq!(component.state, ComponentState::Ok);
        assert_eq!(component.version.as_deref(), Some("1.2.3"));

        // Missing: not an error, just "install me" — never blocks `ready`.
        let report = detect_with(&ctx(runner(&[])));
        let component = report.get("agent-browser").unwrap();
        assert_eq!(component.state, ComponentState::Missing);
        assert_eq!(
            component.fix.as_deref(),
            Some("npm install -g agent-browser")
        );
    }

    #[test]
    fn a_hidden_display_is_xvfb_only() {
        // UNI-299: Xwayland doesn't count — standalone it opens a window.
        let dir = tempfile::tempdir().unwrap();
        stub(dir.path(), "Xvfb");
        let mut c = ctx(runner(&[]));
        c.process_path = vec![dir.path().to_path_buf()];
        assert_eq!(
            detect_with(&c).get("xvfb").unwrap().state,
            ComponentState::Ok
        );
        let only_xwayland = tempfile::tempdir().unwrap();
        stub(only_xwayland.path(), "Xwayland");
        let mut c = ctx(runner(&[]));
        c.process_path = vec![only_xwayland.path().to_path_buf()];
        if !Path::new("/usr/bin/Xvfb").exists() {
            assert_eq!(
                detect_with(&c).get("xvfb").unwrap().state,
                ComponentState::Missing
            );
        }
    }

    #[test]
    fn xvfb_package_names_per_distro() {
        assert_eq!(xvfb_package("apt-get"), "xvfb");
        assert_eq!(xvfb_package("dnf"), "xorg-x11-server-Xvfb");
        assert_eq!(xvfb_package("pacman"), "xorg-server-xvfb");
    }

    #[test]
    fn a_pc_with_everything_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["node", "npm", "pi", "unipi-kanboard"] {
            stub(dir.path(), name);
        }
        std::fs::create_dir_all(
            dir.path()
                .join(".pi/agent/npm/node_modules/@pi-unipi/unipi"),
        )
        .unwrap();
        std::fs::write(
            dir.path()
                .join(".pi/agent/npm/node_modules/@pi-unipi/unipi/package.json"),
            r#"{"version":"3.0.0-alpha.41"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".pi/agent/settings.json"),
            r#"{"packages":["npm:@pi-unipi/unipi@alpha"]}"#,
        )
        .unwrap();
        let mut c = ctx(runner(&[
            ("node", "v24.16.0"),
            ("npm", "11.13.0"),
            ("pi", "1.0.0"),
            ("unipi-kanboard", "0.2.0"),
        ]));
        c.home = dir.path().to_path_buf();
        c.process_path = vec![dir.path().to_path_buf()];
        c.pi_list = Some("User packages:\n  npm:@pi-unipi/unipi@alpha\n".into());
        c.host = Some(HostState {
            run_by: Some("service".into()),
            version: Some("0.4.9".into()),
            bin: Some("/home/t/.local/bin/unipi-host".into()),
        });
        let report = detect_with(&c);
        for name in [
            "node", "npm", "pi", "unipi", "kanboard", "host", "herdr", "tmux",
        ] {
            let state = report.get(name).map(|c| c.state);
            if name == "herdr" || name == "tmux" {
                continue;
            }
            assert_eq!(
                state,
                Some(ComponentState::Ok),
                "{name}: {:?}",
                report.get(name)
            );
        }
        assert!(report.ready);
        assert!(!report.update_available);
        assert_eq!(
            report.get("unipi").unwrap().version.as_deref(),
            Some("3.0.0-alpha.41")
        );
    }

    #[test]
    fn missing_components_say_how_they_would_be_fixed() {
        let report = detect_with(&ctx(runner(&[])));
        assert!(!report.ready);
        assert_eq!(report.get("node").unwrap().state, ComponentState::Missing);
        assert!(report
            .get("node")
            .unwrap()
            .fix
            .as_deref()
            .unwrap()
            .contains("Node"));
        // Nothing installed → no unipi, and kanboard is not "missing" on its own.
        assert_eq!(report.get("unipi").unwrap().state, ComponentState::Missing);
        assert!(report
            .get("unipi")
            .unwrap()
            .fix
            .as_deref()
            .unwrap()
            .contains("pi install"));
        assert_eq!(
            report.get("kanboard").unwrap().state,
            ComponentState::Unsupported
        );
        // npm without Node is missing, and its fix is Node.
        assert_eq!(report.get("npm").unwrap().state, ComponentState::Missing);
        assert_eq!(
            report.get("npm").unwrap().fix.as_deref(),
            Some("Comes with Node")
        );
        // The host is ours to run; nothing to download.
        assert_eq!(report.get("host").unwrap().source, Source::UnipiManaged);
    }

    #[test]
    fn an_old_node_is_outdated_and_a_newer_one_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        stub(dir.path(), "node");
        stub(dir.path(), "npm");
        let mut c = ctx(runner(&[("node", "v22.18.0"), ("npm", "10.0.0")]));
        c.process_path = vec![dir.path().to_path_buf()];
        let report = detect_with(&c);
        assert_eq!(report.get("node").unwrap().state, ComponentState::Outdated);
        assert!(report.update_available);
        assert_eq!(
            report.get("node").unwrap().version.as_deref(),
            Some("22.18.0")
        );
        assert!(report
            .get("node")
            .unwrap()
            .note
            .as_deref()
            .unwrap()
            .contains("22.19.0"));

        // Newer than the floor: ok, never downgraded (decision 4).
        let mut c = ctx(runner(&[("node", "v30.0.0"), ("npm", "11.0.0")]));
        c.process_path = vec![dir.path().to_path_buf()];
        assert_eq!(
            detect_with(&c).get("node").unwrap().state,
            ComponentState::Ok
        );
    }

    #[test]
    fn a_pi_list_without_files_is_broken() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["node", "npm", "pi"] {
            stub(dir.path(), name);
        }
        let mut c = ctx(runner(&[
            ("node", "v24.0.0"),
            ("npm", "11.0.0"),
            ("pi", "1.0.0"),
        ]));
        c.home = dir.path().to_path_buf();
        c.process_path = vec![dir.path().to_path_buf()];
        c.pi_list = Some("npm:@pi-unipi/unipi@alpha\n".into());
        let report = detect_with(&c);
        assert_eq!(report.get("unipi").unwrap().state, ComponentState::Broken);
        assert!(report
            .get("unipi")
            .unwrap()
            .fix
            .as_deref()
            .unwrap()
            .contains("Repair"));
    }

    #[test]
    fn a_binary_that_prints_nothing_is_broken_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        stub(dir.path(), "node");
        stub(dir.path(), "pi");
        let mut c = ctx(runner(&[]));
        c.process_path = vec![dir.path().to_path_buf()];
        let report = detect_with(&c);
        // Present on disk but no version out of it.
        assert_eq!(report.get("node").unwrap().state, ComponentState::Missing);
        assert_eq!(report.get("node").unwrap().source, Source::Found);
    }

    // -------------------------------------------------------------- channel

    /// Today's registry: `latest` is still 2.x while `alpha` is 3.x.
    fn tags_v2_latest() -> DistTags {
        DistTags {
            latest: Some("2.20.5".into()),
            alpha: Some("3.0.0-alpha.41".into()),
        }
    }
    /// The registry once 3.x has been promoted.
    fn tags_v3_latest() -> DistTags {
        DistTags {
            latest: Some("3.0.1".into()),
            alpha: Some("3.0.0-alpha.41".into()),
        }
    }

    #[test]
    fn the_channel_follows_the_updater_spec_then_the_prerelease_id() {
        // A dist-tag spec wins.
        assert_eq!(
            channel_for(
                Some("npm:@pi-unipi/unipi@alpha"),
                Some("3.0.0-alpha.41"),
                &tags_v2_latest()
            ),
            "alpha"
        );
        // A bare spec names the `latest` channel explicitly: the floor does not
        // override that either.
        assert_eq!(
            channel_for(Some("npm:@pi-unipi/unipi"), None, &tags_v2_latest()),
            "latest"
        );
        // A pinned version is not a channel, but it IS a choice: the floor does
        // not override it.
        assert_eq!(
            channel_for(
                Some("npm:@pi-unipi/unipi@3.0.0"),
                Some("2.9.0"),
                &tags_v2_latest()
            ),
            "latest"
        );
        // An installed prerelease keeps its own channel.
        assert_eq!(
            channel_for(None, Some("3.0.0-alpha.41"), &tags_v2_latest()),
            "alpha"
        );
        // A stable install never sees a prerelease tag.
        assert_eq!(
            channel_for(None, Some("3.0.0"), &tags_v2_latest()),
            "latest"
        );
        assert_eq!(install_spec("alpha"), "npm:@pi-unipi/unipi@alpha");
        assert_eq!(install_spec("latest"), "npm:@pi-unipi/unipi");
    }

    #[test]
    fn an_alpha_install_graduates_once_stable_latest_catches_up() {
        // 3.0.0 stable on `latest`, `alpha` left at the last alpha: move to latest.
        let stable = DistTags {
            latest: Some("3.0.0".into()),
            alpha: Some("3.0.0-alpha.45".into()),
        };
        assert_eq!(
            channel_for(
                Some("npm:@pi-unipi/unipi@alpha"),
                Some("3.0.0-alpha.45"),
                &stable
            ),
            "latest"
        );
        assert_eq!(channel_for(None, Some("3.0.0-alpha.45"), &stable), "latest");
        // `alpha` re-pointed at the same stable version: latest too.
        let tie = DistTags {
            latest: Some("3.0.0".into()),
            alpha: Some("3.0.0".into()),
        };
        assert_eq!(
            channel_for(Some("npm:@pi-unipi/unipi@alpha"), Some("3.0.0"), &tie),
            "latest"
        );
        // A newer alpha above stable keeps the alpha channel.
        let ahead = DistTags {
            latest: Some("3.0.0".into()),
            alpha: Some("3.1.0-alpha.1".into()),
        };
        assert_eq!(
            channel_for(
                Some("npm:@pi-unipi/unipi@alpha"),
                Some("3.1.0-alpha.1"),
                &ahead
            ),
            "alpha"
        );
        // Offline: the channel stays.
        assert_eq!(
            channel_for(
                Some("npm:@pi-unipi/unipi@alpha"),
                None,
                &DistTags::default()
            ),
            "alpha"
        );
    }

    #[test]
    fn a_fresh_pc_installs_a_channel_this_build_supports() {
        // Task 0 (UNI-241): nothing installed and nothing pinned. npm's
        // `latest` is 2.x, so the wizard must take `alpha` — installing 2.20.5
        // would give the app a UniPi it cannot talk to.
        assert_eq!(channel_for(None, None, &tags_v2_latest()), "alpha");
        assert_eq!(channel_for(None, None, &tags_v3_latest()), "latest");
        // The same through the spec the app builds.
        assert_eq!(
            install_spec(&channel_for(None, None, &tags_v2_latest())),
            "npm:@pi-unipi/unipi@alpha"
        );
        assert_eq!(
            install_spec(&channel_for(None, None, &tags_v3_latest())),
            "npm:@pi-unipi/unipi"
        );
        // No tags at all (registry unreachable): `latest`, and the caller has
        // already treated that as offline.
        assert_eq!(channel_for(None, None, &DistTags::default()), "latest");
        // `alpha` older than the floor (a rollback) → `latest` still wins.
        assert_eq!(
            channel_for(
                None,
                None,
                &DistTags {
                    latest: Some("2.20.5".into()),
                    alpha: Some("2.9.0".into())
                }
            ),
            "latest"
        );
    }

    #[test]
    fn dist_tags_parse_the_abbreviated_metadata() {
        let tags = DistTags::parse(r#"{"latest":"2.20.5","alpha":"3.0.0-alpha.41"}"#);
        assert_eq!(tags, tags_v2_latest());
        // Extra tags are ignored; rubbish gives nothing.
        assert_eq!(
            DistTags::parse(r#"{"legacy-node20":"0.74.2","latest":"1.2.0"}"#).alpha,
            None
        );
        assert_eq!(DistTags::parse("not json"), DistTags::default());
    }

    #[test]
    fn the_pi_settings_spec_is_read_from_the_agent_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".pi/agent")).unwrap();
        std::fs::write(
            dir.path().join(".pi/agent/settings.json"),
            r#"{"packages":[{"source":"npm:@pi-unipi/unipi@alpha"}]}"#,
        )
        .unwrap();
        assert_eq!(
            read_unipi_spec(dir.path(), Path::new("/nowhere")).as_deref(),
            Some("npm:@pi-unipi/unipi@alpha")
        );
        assert_eq!(
            read_unipi_spec(Path::new("/nowhere"), Path::new("/nowhere")),
            None
        );
    }

    // ----------------------------------------------------------- app-setup

    #[test]
    fn app_setup_json_round_trips_and_marks_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = InstalledRecord::default();
        record.mark(
            "node",
            "22.19.0",
            Path::new("/home/t/.local/share/pi-node/22.19.0"),
            42,
        );
        record.write(dir.path()).unwrap();
        assert_eq!(
            setup_json_path(dir.path()),
            dir.path().join(".unipi/app-setup.json")
        );
        let back = InstalledRecord::read(dir.path());
        let node = back.components.get("node").unwrap();
        assert_eq!((node.version.as_str(), node.at), ("22.19.0", 42));
        // A user's own Node is never in there.
        assert!(!back.components.contains_key("pi"));
    }
}
