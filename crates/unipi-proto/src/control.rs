//! Control (JSON, frame kind 0) message types for `unipi/oino/1`.
//!
//! Mirrors `packages/protocol/src/control.ts`.

use serde::{Deserialize, Serialize};

use crate::blob::{BlobMime, BlobName, BlobPath, BlobRange, BlobSize, Sha256Hex};
use crate::error::{AppErrorDto, ProtoError};
use crate::fs::{FsPath, SearchKind, SearchQuery};
use crate::version::Semver;

/// A validated dimension (cols or rows): integer, 1..=1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(transparent)]
pub struct Dimension(u32);

impl Dimension {
    pub fn value(self) -> u32 {
        self.0
    }
}

impl TryFrom<i64> for Dimension {
    type Error = String;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if (1..=1000).contains(&value) {
            Ok(Dimension(value as u32))
        } else {
            Err(format!("must be an integer 1..=1000, got {value}"))
        }
    }
}

impl TryFrom<u32> for Dimension {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if (1..=1000).contains(&value) {
            Ok(Dimension(value))
        } else {
            Err(format!("must be an integer 1..=1000, got {value}"))
        }
    }
}

impl<'de> Deserialize<'de> for Dimension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = i64::deserialize(deserializer)?;
        Dimension::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A validated pairing code: 6..=64 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PairCode(String);

impl PairCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PairCode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (6..=64).contains(&len) {
            Ok(PairCode(value))
        } else {
            Err(format!("code must be 6..=64 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for PairCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        PairCode::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A validated pairing display name: 1..=64 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PairName(String);

impl PairName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PairName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (1..=64).contains(&len) {
            Ok(PairName(value))
        } else {
            Err(format!("name must be 1..=64 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for PairName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        PairName::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A validated host endpoint identifier: either a 64-char lowercase
/// hex string or a 52-char z-base-32 string (the iroh `NodeId`
/// encodings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HostId(String);

const Z32_ALPHABET: &str = "ybndrfg8ejkmcpqxot1uwisza345h769";

impl HostId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_hex(value: &str) -> bool {
        value.len() == 64
            && value
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    }

    fn is_z32(value: &str) -> bool {
        value.chars().count() == 52 && value.chars().all(|c| Z32_ALPHABET.contains(c))
    }
}

impl TryFrom<String> for HostId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if HostId::is_hex(&value) || HostId::is_z32(&value) {
            Ok(HostId(value))
        } else {
            Err(
                "host_id must be a 64-char lowercase hex string or a 52-char z32 string"
                    .to_string(),
            )
        }
    }
}

impl<'de> Deserialize<'de> for HostId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        HostId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A validated host display name: 1..=64 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HostName(String);

impl HostName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HostName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (1..=64).contains(&len) {
            Ok(HostName(value))
        } else {
            Err(format!("host_name must be 1..=64 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for HostName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        HostName::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// `ok.data` payload for a successful `pair` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairData {
    pub host_id: HostId,
    pub host_name: HostName,
}

/// `ok.data` payload for a successful `notify_config` request (UNI-161 §1):
/// the PC's own ntfy server/topic/token (from `~/.unipi/config/notify/
/// config.json`'s `ntfy` block) plus the per-pairing E2E key (base64,
/// 32 raw bytes) the host generated/loaded from its data dir. `appDetail`
/// mirrors the pi-side `notify.ntfy.appDetail` setting so the app knows
/// whether the host ever encrypts prompt text into the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyConfigData {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token: Option<String>,
    /// base64 (standard, padded) of the 32-byte AES-256-GCM key.
    pub key: String,
    #[serde(default)]
    pub app_detail: NotifyAppDetail,
}

/// Mirrors the pi side's `notify.ntfy.appDetail` ('minimal' | 'full').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyAppDetail {
    #[default]
    Minimal,
    Full,
}

/// A validated title: at most 512 characters (may be empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Title(String);

impl Title {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Title {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if len <= 512 {
            Ok(Title(value))
        } else {
            Err(format!("title must be at most 512 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for Title {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Title::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// The request carried in a `hello` control message. Internally
/// tagged on `op`, matching the TypeScript `RequestSchema`
/// discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Pair {
        code: PairCode,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<PairName>,
    },
    /// Forward one HTTP request to the host's kanboard daemon (see
    /// `crate::board` for the stream layout).
    /// Upload a blob (see `crate::blob`). The body (`size - offset`
    /// bytes) follows in kind-2 frames unless the host answers
    /// `ok{data:{have:true}}` first (dedup: already has this sha256).
    BlobPut {
        sha256: Sha256Hex,
        size: BlobSize,
        mime: BlobMime,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<BlobName>,
        /// Resume: bytes already held by the host from a previous attempt.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        offset: Option<u64>,
    },
    /// Download a blob by sha256 (host blobs dir) or by an allow-listed
    /// path (see `crate::blob`). The host streams `ok{data:{size,mime}}`
    /// then the bytes in kind-1 frames, then `exit{code:0}`.
    BlobGet {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        sha256: Option<Sha256Hex>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        path: Option<BlobPath>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        range: Option<BlobRange>,
    },
    /// Suggested roots to browse: live pi/herdr pane cwds plus `$HOME`.
    FsRoots,
    /// Lists one directory's entries (UNI-155 "files").
    FsList {
        path: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        hidden: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Reads a text file; the bytes follow in kind-1 frames (see `crate::fs`).
    FsRead {
        path: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        range: Option<BlobRange>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
        /// UNI-191: when true, a file that would otherwise refuse with
        /// `E_BINARY` is instead lossily decoded as UTF-8 and returned
        /// ("Edit as text" for any file type), still capped by
        /// `FS_READ_CAP`.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        as_text: Option<bool>,
    },
    /// Writes a text file atomically; refuses with `E_CONFLICT` if
    /// `base_hash` no longer matches what's on disk.
    FsWrite {
        path: FsPath,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        base_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        create: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Stats one path without reading its content.
    FsStat {
        path: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Finds files by name or content under `root` (fd/rg if on `PATH`,
    /// else a bounded walk).
    FsSearch {
        root: FsPath,
        query: SearchQuery,
        kind: SearchKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Creates a directory (and parents).
    FsMkdir {
        path: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Renames/moves a path.
    FsRename {
        from: FsPath,
        to: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// UNI-199: copies an already-uploaded blob (`blob_put`) to a
    /// filesystem path, so the phone can "upload" a file to a chosen
    /// folder without a separate download/stream round trip.
    FsImport {
        sha256: Sha256Hex,
        to: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        overwrite: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// Deletes a file or (recursively) a directory. Requires `confirm:true`;
    /// refuses to delete a root or `$HOME`.
    FsDelete {
        path: FsPath,
        #[serde(default)]
        confirm: bool,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// UNI-217: basic git on the PC for the repo containing `path` (same
    /// scope as `fs_*`); `cmd` says what (see `crate::git`).
    Git {
        path: FsPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
        cmd: crate::git::GitCmd,
    },
    /// UNI-161 §1: "Get from this PC" — returns the host's own ntfy
    /// server/topic/token plus the per-pairing E2E key (`NotifyConfigData`).
    NotifyConfig,
    /// UNI-161 §2: registers (or refreshes) this phone's UnifiedPush
    /// endpoint URL so the pi-side notify fan-out can reach it directly.
    NotifyRegister {
        endpoint: NotifyEndpoint,
    },
    /// UNI-161 §6: sends one encrypted test notification through the same
    /// ntfy config `notify_config` reads, proving the whole path end to end.
    NotifyTest,
    /// UNI-213: listening loopback TCP ports on the host (`PortsListData`).
    PortsList,
    /// UNI-213: raw bidirectional pipe to `127.0.0.1:port` on the host
    /// (see `crate::forward`).
    TcpForward {
        port: crate::forward::ForwardPort,
    },
    /// UNI-214: files pushed from the PC (`unipi-host push`), long-poll.
    PushList {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        wait: Option<crate::push::PushWait>,
    },
    /// UNI-214: what the phone did with a push.
    PushDone {
        id: crate::push::PushId,
        outcome: crate::push::PushOutcome,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
    },
    /// UNI-215: the host's dedicated browser — found/enabled/running + tabs.
    BrowserStatus,
    /// UNI-215: stream a tab of the host's dedicated browser (see `crate::browser`).
    BrowserOpen {
        viewport: crate::browser::BrowserViewport,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        url: Option<crate::browser::BrowserUrl>,
        /// Show this tab (id from `browser_status`); default: the last used one.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        target: Option<String>,
        /// UNI-292 (1.16.0): the browser profile to show; default `"default"`.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        profile: Option<crate::browser::BrowserProfile>,
    },
    /// UNI-215: shut the host's dedicated browser down now.
    BrowserClose,
    /// UNI-268 (proto 1.13.0): show the dedicated browser's window on the PC
    /// (`show: true`) or run it without one. Saved like the owner's
    /// "Show its window on this PC"; a running browser relaunches with the
    /// same tabs. `ok{data: BrowserStatusData}`.
    BrowserWindow {
        show: bool,
    },
    /// UNI-296 (proto 1.17.0): the host's browser profiles (read only).
    /// `ok{data: BrowserProfilesData}`.
    BrowserProfiles,
    /// UNI-296 (proto 1.17.0): take over a profile's browser from agents
    /// (`on: true`: their `Input.*`/navigation is refused) or give it back.
    /// `ok{data: BrowserAgent}`.
    BrowserTakeover {
        on: bool,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        profile: Option<crate::browser::BrowserProfile>,
    },
    /// UNI-216: can this PC stream its desktop to this device?
    /// UNI-248: ask the PC's owner to switch a feature on for this device
    /// (stored as pending; never enabled by itself). → `FeatureRequestItem`.
    FeatureRequest {
        feature: crate::sharing::FeatureKind,
        #[serde(
            default,
            skip_serializing_if = "crate::sharing::FeatureScope::is_empty"
        )]
        scope: crate::sharing::FeatureScope,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        note: Option<crate::sharing::FeatureNote>,
    },
    /// UNI-248: this device's latest request per feature. → `FeatureRequestsData`.
    FeatureRequests,
    /// UNI-253: this device turns a feature off again (never on). `scope`
    /// defaults to `self` (just this device); `all` switches the feature off
    /// on the PC, and only a device that has it may ask for that.
    /// → `FeatureLeaveData`.
    FeatureLeave {
        feature: crate::sharing::FeatureKind,
        #[serde(
            default,
            skip_serializing_if = "crate::sharing::FeatureLeaveScope::is_self"
        )]
        scope: crate::sharing::FeatureLeaveScope,
    },
    /// UNI-260: start pi in a new herdr tab / tmux window. → `ChatNewData`.
    /// UNI-232: local-only (this PC's own app over `app.sock`; iroh peers get
    /// E_UNAUTHORIZED). What runs here. → `HostInfoData`.
    HostInfo,
    /// UNI-232: local-only. Start a phone pairing. → `PairBeginData`.
    PairBegin {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        ttl_secs: Option<crate::admin::PairTtl>,
    },
    /// UNI-232: local-only. → `PairStatusData`.
    PairStatus {
        code: crate::admin::PairCodeRef,
        /// Close the attempt (the dialog was closed) unless it already paired.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cancel: bool,
    },
    /// UNI-232: local-only. → `DevicesListData`.
    DevicesList,
    /// UNI-232: local-only. Unpair a phone. → `ok{}`.
    DeviceRevoke {
        id: crate::admin::DeviceIdRef,
    },
    /// UNI-317: every job (name/schedule/next/last status/enabled) plus
    /// whether `cron pause` is in effect. → `CronJobsData`.
    CronList,
    /// UNI-317: one job's detail plus its run history. → `CronShowData`.
    CronShow {
        id: crate::cron::CronJobId,
    },
    /// UNI-317: run history, newest first; every job's when `id` is
    /// omitted. → `CronRunsData`.
    CronRuns {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        id: Option<crate::cron::CronJobId>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        limit: Option<crate::cron::CronRunsLimit>,
    },
    /// UNI-317: one run's captured output (found by run id alone). → `CronLogData`.
    CronLog {
        run_id: crate::cron::CronRunId,
    },
    /// UNI-317: add a job. → `CronJob`.
    CronAdd(crate::cron::CronAddRequest),
    /// UNI-317: patch a job (only given fields change). → `CronJob`.
    CronEdit {
        id: crate::cron::CronJobId,
        patch: crate::cron::CronEditRequest,
    },
    /// UNI-317: delete a job (and its run history). → `ok{data:null}`.
    CronRm {
        id: crate::cron::CronJobId,
    },
    CronEnable {
        id: crate::cron::CronJobId,
    },
    CronDisable {
        id: crate::cron::CronJobId,
    },
    /// UNI-317: run now, bypassing the schedule. → `CronRunStartedData`.
    CronRun {
        id: crate::cron::CronJobId,
    },
    /// UNI-317: kill a running run. → `CronStoppedData`.
    CronStop {
        id: crate::cron::CronJobId,
    },
    /// UNI-317: stop new runs starting; running ones finish. → `ok{data:null}`.
    #[serde(rename = "cron_pause")]
    CronPauseAll,
    #[serde(rename = "cron_resume")]
    CronResumeAll,
    /// UNI-317: next fire, running jobs, paused. → `CronStatusData`.
    CronStatus,
    /// UNI-317: the add/edit form's live preview — resolves plain
    /// language or a machine schedule to its 6-field cron (or `at:`) form
    /// plus the next [`crate::cron::CRON_PARSE_PREVIEW`] fires.
    /// → `CronParseData`.
    CronParse {
        /// A recurring schedule (plain language or 6-field cron).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        schedule: Option<String>,
        /// A one-time schedule (plain language or RFC3339/`at:`),
        /// instead of `schedule`.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        tz: Option<String>,
    },
    /// UNI-319: open a run's saved session as a live chat: a pi already on
    /// it, else `<launcher> --session <file>` started at `target` in the
    /// job's folder. → `CronOpenSessionData`.
    /// UNI-318: the `cron` block of `config.json`. → `CronSettingsData`.
    CronSettingsGet,
    /// UNI-318: patch the `cron` block (validated). → `CronSettingsData`.
    CronSettingsSet(crate::cron::CronSettingsSetRequest),
    /// host.md §9.3 (WP-H6): open a new terminal — a host-owned PTY — in
    /// `cwd` (its workspace). Answers `ok{data:TermAttachData}` and then the
    /// stream carries output (kind 1), input (kind 2) and the `resize` /
    /// `exit` / `title` controls, exactly like `term_attach`.
    TermOpen {
        cwd: FsPath,
        cols: Dimension,
        rows: Dimension,
        /// Shell to run; default `$SHELL -l` (login shell in `cwd`).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        shell: Option<crate::term::TermShell>,
    },
    /// host.md §9.3 (WP-H6): re-attach an open stream to an existing PTY
    /// (resizes it through the host's resize throttle, then streams on).
    /// New output only — the host keeps no scrollback.
    /// → `ok{data:TermAttachData}` then the stream.
    TermAttach {
        term_id: crate::term::TermId,
        cols: Dimension,
        rows: Dimension,
    },
    /// host.md §9.3 (WP-H6): every terminal the host holds, live or
    /// exited-and-not-yet-killed. → `ok{data:[TermInfo, …]}`.
    TermList,
    /// host.md §9.3 (WP-H6): close a terminal (kill its child, forget the
    /// id). Idempotent. → `ok{data:null}`.
    TermKill {
        term_id: crate::term::TermId,
    },
    /// host.md §7.2/§4.2 (WP-H4): every session the host knows, newest activity
    /// first — a lazy list (headers only, no runtime loaded) merged with the
    /// host's live cards. → `[SessionCard, …]` (bare array, like `term_list`).
    SessionsList,
    /// host.md §7.4 (WP-H4): the one per-connection hub event stream. The host
    /// answers `ok{data:null}`, then pushes `session_changed` / `session_removed`
    /// / `host_notice` / `ui` events (a retained safe-mode notice first) until
    /// the stream closes. One per connection, not per session.
    EventsOpen,
    /// host.md §4.2/§7.3 (WP-H4): a new session (a fresh runtime over a new
    /// session file; written on its first message). Answers
    /// `ok{data:SessionSnapshot}` and keeps the bi-stream as the session's
    /// stream, exactly like `session_open`.
    SessionNew {
        /// Project working directory; default: the host's primary cwd.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cwd: Option<FsPath>,
        /// Display name (`session_info` entry).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<crate::session::SessionName>,
    },
    /// host.md §7.3 (WP-H4): open (or resume) a session as a live stream. Loads
    /// or reuses the runtime (takeover per §8) and answers
    /// `ok{data:SessionSnapshot}`: a full snapshot (`reset:true`) or, when the
    /// hub still buffers everything after `since_seq`, a resume patch
    /// (`reset:false, replay:[…]`).
    SessionOpen {
        /// Exact id, unique prefix (≥ 4 chars) or session file path.
        session: crate::session::SessionTarget,
        /// Last event seq the client saw; absent = full snapshot.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        since_seq: Option<u64>,
    },
    /// host.md §4.2/§7.3 (WP-H4): fork a session (before `entry_id`, pi
    /// semantics; at the leaf when absent) and keep the fork as this stream's
    /// session. Answers `ok{data:SessionSnapshot}` for the fork.
    SessionFork {
        /// The session to fork.
        session: crate::session::SessionTarget,
        /// Fork before this entry; absent = at the leaf (a clone).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        entry_id: Option<String>,
        /// Name of the fork.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<crate::session::SessionName>,
    },
    /// host.md §4.2 (WP-H4): move the session's file to `<agent dir>/trash/`
    /// (restorable by hand / from the archived list). → the archived card.
    SessionArchive {
        /// Exact id, unique prefix or session file path.
        session: crate::session::SessionTarget,
    },
    /// host.md §7.2 (WP-H4): set the session's display name (wakes the runtime
    /// to write the `session_info` entry, then emits a card update). → `null`.
    SessionRename {
        /// Exact id, unique prefix or session file path.
        session: crate::session::SessionTarget,
        /// The new name.
        name: crate::session::SessionName,
    },
    /// host.md §4.2 (WP-H4): dispose the runtime when idle and no other
    /// subscriber; the session file stays. → `null`.
    SessionClose {
        /// Session id (or a unique prefix / file path).
        session: crate::session::SessionTarget,
    },
    /// v2 (host.md §9.9): the host settings object (`config.json`), secrets
    /// masked. Any paired device may read it; → the object.
    HostSettingsGet,
    /// v2 (host.md §9.9): local-only. Patch one dotted key (validated; an
    /// unknown key or a wrong-typed value is `E_SCHEMA`). → the masked,
    /// complete object.
    HostSettingsSet(crate::settings::HostSettingsSetRequest),
    /// App spec §3.14 (WP-A22; ext-abi §5.5, G-18): run the `unipi-kanboard`
    /// CLI once — `--json`, actor `user` pinned by the host, subcommand
    /// allow-list — and answer with the decoded outcome
    /// (`ok{data:BoardCliData}`): the stdout payload, or the stderr
    /// `{ok:false, error, kind}` line as a domain failure.
    BoardCli {
        /// The subcommand and its arguments (validated `BoardArg`s).
        #[serde(with = "crate::board::serde_args", default)]
        args: Vec<String>,
        /// `--project <slug>`, pinned per view.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        project: Option<crate::board::BoardSlug>,
    },
    /// App spec §3.14 (WP-A22): turn the board file watcher on or off
    /// (refcounted). While on, board file changes emit
    /// `HubEvent::BoardChanged{project}` on every open `events_open` stream.
    /// → `ok{data:BoardWatchData}`.
    BoardWatch {
        #[serde(default)]
        on: bool,
    },
}

/// A UnifiedPush/ntfy endpoint URL: 1..=2048 characters, must look like an
/// `https://` URL (checked loosely here; the host does the real request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NotifyEndpoint(String);

impl NotifyEndpoint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NotifyEndpoint {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 2048 {
            return Err(format!(
                "endpoint must be 1..=2048 characters, got {}",
                value.len()
            ));
        }
        if !value.starts_with("https://") {
            return Err("endpoint must be an https:// URL".to_string());
        }
        Ok(NotifyEndpoint(value))
    }
}

impl<'de> Deserialize<'de> for NotifyEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        NotifyEndpoint::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// The control (JSON, kind 0) message envelope for `unipi/oino/1`.
/// Internally tagged on `control`, matching the TypeScript
/// `ControlSchema` discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Hello {
        proto: Semver,
        req: Request,
    },
    Ok {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        data: Option<serde_json::Value>,
    },
    Err {
        error: AppErrorDto,
    },
    Resize {
        cols: Dimension,
        rows: Dimension,
    },
    Exit {
        code: Option<i64>,
    },
    Title {
        title: Title,
    },
    /// UNI-215: phone → host input/navigation on a `browser_open` stream.
    BrowserCmd {
        cmd: crate::browser::BrowserCmd,
    },
    /// UNI-215: host → phone event on a `browser_open` stream (a `frame`
    /// event is followed by one kind-1 frame with the JPEG).
    BrowserEvent {
        event: crate::browser::BrowserEvent,
    },
    /// host.md §7.3/§7.4 (WP-H4): one flush tick's batch of session events (≤ 1
    /// frame per 16 ms per stream; delta events already coalesced).
    Events {
        items: Vec<crate::session::SessionEventDto>,
    },
    /// host.md §7.3 (WP-H4): any oino RPC command verbatim on a session stream,
    /// correlated by the opaque `id` (echoed in the `ok`/`err` reply, after the
    /// events the command caused).
    SessionCmd {
        /// Correlation id, echoed verbatim (`Option<serde_json::Value>`: string or number).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        id: Option<serde_json::Value>,
        /// The command (`RpcRequest` JSON: pi commands + `_oino/*`).
        cmd: serde_json::Value,
    },
}

impl Control {
    pub fn validate(&self) -> Result<(), ProtoError> {
        let value = serde_json::to_value(self).map_err(|error| ProtoError::Schema {
            message: error.to_string(),
            path: vec![],
            detail: None,
        })?;
        serde_path_to_error::deserialize::<_, Control>(value)
            .map(|_| ())
            .map_err(ProtoError::from_path_error)?;
        // v2 (host.md §7.3): a session command is an RPC command object with a `type`; the
        // host forwards it verbatim and rejects anything else before it reaches a runtime.
        match self {
            Self::SessionCmd { cmd, .. }
                if !cmd.is_object() || cmd.get("type").and_then(|t| t.as_str()).is_none() =>
            {
                Err(ProtoError::Schema {
                    message: "cmd must be an RPC command object with a type".to_string(),
                    path: vec![crate::error::PathSegment::Key("cmd".to_string())],
                    detail: None,
                })
            }
            Self::Events { items } if items.iter().any(|item| item.seq == 0) => {
                Err(ProtoError::Schema {
                    message: "session events are numbered from 1".to_string(),
                    path: vec![crate::error::PathSegment::Key("items".to_string())],
                    detail: None,
                })
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_pair_round_trips() {
        let json = r#"{"type":"hello","proto":"2.0.0","req":{"op":"pair","code":"123456"}}"#;
        let parsed: Control = serde_json::from_str(json).expect("parse");
        match parsed {
            Control::Hello { proto, req } => {
                assert_eq!(proto.as_str(), "2.0.0");
                match req {
                    Request::Pair { code, name } => {
                        assert_eq!(code.as_str(), "123456");
                        assert_eq!(name, None);
                    }
                    _ => panic!("expected Pair"),
                }
            }
            _ => panic!("expected Hello"),
        }
    }

    #[test]
    fn unknown_extra_fields_are_ignored() {
        let json = r#"{"type":"ok","data":null,"extra":"ignored"}"#;
        let parsed: Result<Control, _> = serde_json::from_str(json);
        assert!(parsed.is_ok());
    }

    #[test]
    fn exit_accepts_null_code() {
        let json = r#"{"type":"exit","code":null}"#;
        let parsed: Control = serde_json::from_str(json).expect("parse");
        match parsed {
            Control::Exit { code } => assert_eq!(code, None),
            _ => panic!("expected Exit"),
        }
    }

    #[test]
    fn title_rejects_over_512_chars() {
        let title = "t".repeat(513);
        let json = format!(r#"{{"type":"title","title":"{title}"}}"#);
        let parsed: Result<Control, _> = serde_json::from_str(&json);
        assert!(parsed.is_err());
    }
}
