//! Control (JSON, frame kind 0) message types for `unipi/app/1`.
//!
//! Mirrors `packages/protocol/src/control.ts`.

use serde::{Deserialize, Serialize};

use crate::blob::{BlobMime, BlobName, BlobPath, BlobRange, BlobSize, Sha256Hex};
use crate::board::{BoardMethod, BoardPath, BoardProject, BodyLen, ContentType};
use crate::error::{AppErrorDto, ProtoError};
use crate::fs::{FsPath, SearchKind, SearchQuery};
use crate::version::Semver;

/// Multiplexer kind: herdr (first-party) or tmux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MuxKind {
    Herdr,
    Tmux,
}

/// How the terminal reacts to the phone's own viewport size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizePolicy {
    FitPhone,
    KeepPc,
}

/// A validated session identifier: 1..=128 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (1..=128).contains(&len) {
            Ok(SessionId(value))
        } else {
            Err(format!("session must be 1..=128 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SessionId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

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

/// A validated session display name (as returned in `ListData`):
/// 1..=128 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionName(String);

impl SessionName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (1..=128).contains(&len) {
            Ok(SessionName(value))
        } else {
            Err(format!("name must be 1..=128 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for SessionName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SessionName::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// One session entry returned by a `list` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub name: SessionName,
    pub running: bool,
    pub default: bool,
}

/// `ok.data` payload for a successful `list` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListData {
    pub sessions: Vec<SessionSummary>,
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

/// `ok.data` payload for a successful `attach` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachData {
    pub effective_size_policy: SizePolicy,
}

/// Validated `lines` for a `preview` request: 1..=200.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Lines(u16);

impl Lines {
    pub fn value(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Lines {
    type Error = String;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=200).contains(&value) {
            Ok(Lines(value))
        } else {
            Err(format!("lines must be 1..=200, got {value}"))
        }
    }
}

impl<'de> Deserialize<'de> for Lines {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        Lines::try_from(raw).map_err(serde::de::Error::custom)
    }
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

/// A herdr identifier (`w4E`, `w4E:t16`, `w4E:p18`): 1..=128 chars,
/// no whitespace or control characters (mirrors `HerdrIdSchema`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HerdrId(String);

impl HerdrId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HerdrId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if !(1..=128).contains(&len) {
            return Err(format!("herdr id must be 1..=128 characters, got {len}"));
        }
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("herdr id must not contain whitespace or control characters".to_string());
        }
        Ok(HerdrId(value))
    }
}

impl<'de> Deserialize<'de> for HerdrId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        HerdrId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A tmux identifier with its kind: `$3` (session), `@5` (window) or
/// `%7` (pane). One type covers all three so a struct can name the
/// field, and the regex pins the shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TmuxId<const KIND: char>(String);

impl<const KIND: char> TmuxId<KIND> {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const KIND: char> TryFrom<String> for TmuxId<KIND> {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let digits = value
            .strip_prefix(KIND)
            .ok_or_else(|| format!("tmux id must start with {KIND}, got {value:?}"))?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!(
                "tmux id must look like {KIND}<digits>, got {value:?}"
            ));
        }
        Ok(TmuxId(value))
    }
}

impl<'de, const KIND: char> Deserialize<'de> for TmuxId<KIND> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        TmuxId::<KIND>::try_from(raw).map_err(serde::de::Error::custom)
    }
}

pub type TmuxSessionId = TmuxId<'$'>;
pub type TmuxWindowId = TmuxId<'@'>;
pub type TmuxPaneId = TmuxId<'%'>;

/// Where in a multiplexer to attach/focus/preview (DESIGN.md §5.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mux", rename_all = "snake_case")]
pub enum AttachTarget {
    Herdr {
        session: SessionId,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        workspace_id: Option<HerdrId>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        tab_id: Option<HerdrId>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pane_id: Option<HerdrId>,
    },
    Tmux {
        session: SessionId,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        window_id: Option<TmuxWindowId>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pane_id: Option<TmuxPaneId>,
    },
}

/// The pi (or other agent) bound to one pane (DESIGN.md §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneAgent {
    pub name: String,
    pub status: AgentStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session: Option<AgentSessionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionRef {
    pub kind: String,
    pub value: String,
}

/// herdr's own status enum is `idle|working|blocked|done|unknown`
/// (`herdr api schema --json`, `AgentStatus`), which is exactly our
/// proto enum, so the mapping is identity; anything unrecognised
/// maps to `unknown` (see `agent_status_from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    /// Maps an arbitrary herdr status string onto the proto enum.
    /// Unrecognised values become `Unknown`.
    pub fn from_herdr(raw: &str) -> Self {
        match raw {
            "idle" => AgentStatus::Idle,
            "working" => AgentStatus::Working,
            "blocked" => AgentStatus::Blocked,
            "done" => AgentStatus::Done,
            _ => AgentStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrPane {
    pub id: HerdrId,
    pub focused: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub terminal_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent: Option<PaneAgent>,
    /// The chat bridge of the pi in this pane, when one is reachable
    /// (`chat_list` only; DESIGN §3.4).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chat: Option<ChatBridgeRef>,
}

/// A pi chat bridge found on the host (`~/.unipi/bridge/<pid>.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBridgeRef {
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    /// Set while this pi is waiting on the user (UNI-161 "needs you"): an
    /// open dialog, or idle right after an `agent_end` the phone hasn't seen
    /// yet. Cleared once answered/opened.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub waiting: Option<ChatWaiting>,
    /// pi's own run state from the bridge record (UNI-212) — herdr's pane
    /// `agent_status` can miss a run; absent from older bridges.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub running: Option<bool>,
    /// UNI-221: wake-capable work still pending while pi is idle (a bg task
    /// that will wake it, a background subagent, a fusion handoff) — e.g.
    /// "bg: npm test · 2 subagents working". The app shows "Working…", not
    /// idle/finished. Absent when nothing is pending / older bridges.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pending_work: Option<String>,
    /// UNI-269: the long-horizon working mode (`goal`, `ralph`, `swarm`,
    /// `graph`) — the list's mode badge. Absent = regular / older bridges.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lh_mode: Option<String>,
    /// UNI-269: the mode of a parked long-horizon owner ("Graph · paused").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lh_paused: Option<String>,
}

/// What a waiting pi is waiting on (`BridgeRecord.waiting` → `ChatBridgeRef.waiting`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatWaiting {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    /// ms since epoch.
    pub since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrTab {
    pub id: HerdrId,
    pub label: String,
    pub number: u32,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub panes: Vec<HerdrPane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrWorkspace {
    pub id: HerdrId,
    pub label: String,
    pub number: u32,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub tab_count: u32,
    pub pane_count: u32,
    pub tabs: Vec<HerdrTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrSession {
    pub name: SessionName,
    pub running: bool,
    pub default: bool,
    pub workspaces: Vec<HerdrWorkspace>,
    /// Set when this session's snapshot could not be read.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HerdrState {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sessions: Option<Vec<HerdrSession>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxPane {
    pub id: TmuxPaneId,
    pub index: u32,
    pub active: bool,
    pub current_command: String,
    pub cwd: String,
    pub pid: u32,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent: Option<PaneAgent>,
    /// UNI-260: the pi chat bridge running in this pane (`chat_list` only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chat: Option<ChatBridgeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxWindow {
    pub id: TmuxWindowId,
    pub index: u32,
    pub name: String,
    pub active: bool,
    pub panes: Vec<TmuxPane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxSession {
    pub id: TmuxSessionId,
    pub name: String,
    pub attached: u32,
    pub window_count: u32,
    pub created: u64,
    pub activity: u64,
    pub windows: Vec<TmuxWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TmuxState {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sessions: Option<Vec<TmuxSession>>,
}

fn validate_state(available: bool, reason: &Option<String>, sessions: &bool) -> Result<(), String> {
    if available {
        if reason.is_some() {
            return Err("available state must not carry a reason".into());
        }
        if !*sessions {
            return Err("available state must carry sessions".into());
        }
    } else if reason.is_none() || *sessions {
        return Err("unavailable state must carry a reason and no sessions".into());
    }
    Ok(())
}

impl<'de> Deserialize<'de> for HerdrState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            available: bool,
            #[serde(default)]
            reason: Option<String>,
            #[serde(default)]
            sessions: Option<Vec<HerdrSession>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        validate_state(raw.available, &raw.reason, &raw.sessions.is_some())
            .map_err(serde::de::Error::custom)?;
        Ok(HerdrState {
            available: raw.available,
            reason: raw.reason,
            sessions: raw.sessions,
        })
    }
}

impl<'de> Deserialize<'de> for TmuxState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            available: bool,
            #[serde(default)]
            reason: Option<String>,
            #[serde(default)]
            sessions: Option<Vec<TmuxSession>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        validate_state(raw.available, &raw.reason, &raw.sessions.is_some())
            .map_err(serde::de::Error::custom)?;
        Ok(TmuxState {
            available: raw.available,
            reason: raw.reason,
            sessions: raw.sessions,
        })
    }
}

/// `ok.data` for a `tree` request (DESIGN.md §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeData {
    /// Session names the host protects (`--protect`); the app dims
    /// their rows and refuses to open them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected: Vec<SessionName>,
    pub herdr: HerdrState,
    pub tmux: TmuxState,
}

/// ANSI-kept pane text for a `preview` reply, capped at 64 KiB
/// (mirrors `PreviewDataSchema`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PreviewText(String);

impl PreviewText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for PreviewText {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() <= 64 * 1024 {
            Ok(PreviewText(value))
        } else {
            Err(format!(
                "preview text must be at most 64 KiB, got {} bytes",
                value.len()
            ))
        }
    }
}

impl<'de> Deserialize<'de> for PreviewText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        PreviewText::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// `ok.data` for a `preview` request: pane text with ANSI kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewData {
    pub text: PreviewText,
}

/// One entry of the shortcuts panel: a named action and the key
/// (with prefix shown separately) that triggers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutKey {
    pub name: String,
    pub key: String,
}

/// The shortcuts panel data for one multiplexer (DESIGN.md §3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutMux {
    pub prefix: String,
    pub keys: Vec<ShortcutKey>,
}

/// Reply payload for the `shortcuts` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutsData {
    pub herdr: ShortcutMux,
    pub tmux: ShortcutMux,
}

/// A multiplexer tree mutation or navigation, sent as a `mux_action`
/// request. Tagged on `action`; herdr and tmux pane cycling get
/// mux-prefixed tags because the payloads would otherwise be
/// identical and a session name can exist in both multiplexers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MuxAction {
    WorkspaceCreate {
        session: SessionName,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<SessionName>,
    },
    WorkspaceRename {
        session: SessionName,
        workspace_id: HerdrId,
        label: SessionName,
    },
    WorkspaceClose {
        session: SessionName,
        workspace_id: HerdrId,
    },
    TabNext {
        session: SessionName,
    },
    TabPrev {
        session: SessionName,
    },
    HerdrPaneNext {
        session: SessionName,
    },
    HerdrPanePrev {
        session: SessionName,
    },
    TmuxSessionCreate {
        name: SessionName,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    TmuxSessionRename {
        session: SessionName,
        name: SessionName,
    },
    TmuxSessionKill {
        session: SessionName,
    },
    WindowNext {
        session: SessionName,
    },
    WindowPrev {
        session: SessionName,
    },
    TmuxPaneNext {
        session: SessionName,
    },
    TmuxPanePrev {
        session: SessionName,
    },
}

impl MuxAction {
    /// The session the action mutates or navigates in; protected
    /// sessions reject every action.
    pub fn session(&self) -> &str {
        match self {
            MuxAction::WorkspaceCreate { session, .. }
            | MuxAction::WorkspaceRename { session, .. }
            | MuxAction::WorkspaceClose { session, .. }
            | MuxAction::TabNext { session }
            | MuxAction::TabPrev { session }
            | MuxAction::HerdrPaneNext { session }
            | MuxAction::HerdrPanePrev { session }
            | MuxAction::TmuxSessionRename { session, .. }
            | MuxAction::TmuxSessionKill { session }
            | MuxAction::WindowNext { session }
            | MuxAction::WindowPrev { session }
            | MuxAction::TmuxPaneNext { session }
            | MuxAction::TmuxPanePrev { session } => session.as_str(),
            MuxAction::TmuxSessionCreate { name, .. } => name.as_str(),
        }
    }
}

/// The request carried in a `hello` control message. Internally
/// tagged on `op`, matching the TypeScript `RequestSchema`
/// discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    List {
        mux: MuxKind,
    },
    Attach {
        target: AttachTarget,
        cols: Dimension,
        rows: Dimension,
        size_policy: SizePolicy,
    },
    Tree,
    Shortcuts,
    MuxAction {
        #[serde(flatten)]
        action: MuxAction,
    },
    Shell {
        cols: Dimension,
        rows: Dimension,
    },
    Preview {
        target: AttachTarget,
        lines: Lines,
    },
    Pair {
        code: PairCode,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<PairName>,
    },
    /// Forward one HTTP request to the host's kanboard daemon (see
    /// `crate::board` for the stream layout).
    BoardHttp {
        method: BoardMethod,
        path: BoardPath,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        content_type: Option<ContentType>,
        #[serde(skip_serializing_if = "BodyLen::is_zero", default)]
        body_len: BodyLen,
    },
    /// Subscribe to a project's kanboard SSE revision stream.
    BoardSse {
        project: BoardProject,
    },
    /// The herdr tree with each pi pane's chat bridge (DESIGN §3.4).
    ChatList,
    /// Open a live chat with the pi whose bridge has this pid; then
    /// `chat{line}` controls both ways (docs/m5/PROTOCOL.md).
    ChatOpen {
        pid: u32,
    },
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
    BrowserWindow { show: bool },
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
    DesktopStatus,
    /// UNI-216: stream the PC's desktop (H.264) with control (see `crate::desktop`).
    DesktopOpen {
        viewport: crate::desktop::DesktopViewport,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        quality: Option<crate::desktop::DesktopQuality>,
        /// Ask for view-only even when control is allowed.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        view_only: Option<bool>,
        /// UNI-271: which monitor (`desktop_status.displays[].id`); default primary.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        display: Option<String>,
    },
    /// UNI-248: ask the PC's owner to switch a feature on for this device
    /// (stored as pending; never enabled by itself). → `FeatureRequestItem`.
    FeatureRequest {
        feature: crate::sharing::FeatureKind,
        #[serde(default, skip_serializing_if = "crate::sharing::FeatureScope::is_empty")]
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
        #[serde(default, skip_serializing_if = "crate::sharing::FeatureLeaveScope::is_self")]
        scope: crate::sharing::FeatureLeaveScope,
    },
    /// UNI-260: start pi in a new herdr tab / tmux window. → `ChatNewData`.
    ChatNew {
        target: crate::chat::ChatNewTarget,
        /// Folder pi starts in (Files scope); `~` = home. Default: home.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cwd: Option<FsPath>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        outside: Option<bool>,
    },
    /// UNI-260: stop a pi (and close its tab/window). → `ChatCloseData`.
    ChatClose {
        pid: std::num::NonZeroU32,
        #[serde(default)]
        close_tab: bool,
    },
    /// UNI-260: the PC's chat launcher. → `ChatConfigData`.
    ChatConfig,
    /// UNI-260: set the launcher (`""` = default). → `ChatConfigData`.
    ChatConfigSet {
        launcher: crate::chat::ChatLauncher,
    },
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
    CronShow { id: crate::cron::CronJobId },
    /// UNI-317: run history, newest first; every job's when `id` is
    /// omitted. → `CronRunsData`.
    CronRuns {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        id: Option<crate::cron::CronJobId>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        limit: Option<crate::cron::CronRunsLimit>,
    },
    /// UNI-317: one run's captured output (found by run id alone). → `CronLogData`.
    CronLog { run_id: crate::cron::CronRunId },
    /// UNI-317: add a job. → `CronJob`.
    CronAdd(crate::cron::CronAddRequest),
    /// UNI-317: patch a job (only given fields change). → `CronJob`.
    CronEdit {
        id: crate::cron::CronJobId,
        patch: crate::cron::CronEditRequest,
    },
    /// UNI-317: delete a job (and its run history). → `ok{data:null}`.
    CronRm { id: crate::cron::CronJobId },
    CronEnable { id: crate::cron::CronJobId },
    CronDisable { id: crate::cron::CronJobId },
    /// UNI-317: run now, bypassing the schedule. → `CronRunStartedData`.
    CronRun { id: crate::cron::CronJobId },
    /// UNI-317: kill a running run. → `CronStoppedData`.
    CronStop { id: crate::cron::CronJobId },
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
    CronOpenSession {
        run_id: crate::cron::CronRunId,
        target: crate::chat::ChatNewTarget,
    },
    /// UNI-318: the `cron` block of `config.json`. → `CronSettingsData`.
    CronSettingsGet,
    /// UNI-318: patch the `cron` block (validated). → `CronSettingsData`.
    CronSettingsSet(crate::cron::CronSettingsSetRequest),
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
            return Err(format!("endpoint must be 1..=2048 characters, got {}", value.len()));
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

/// The control (JSON, kind 0) message envelope for `unipi/app/1`.
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
    /// One bridge message (a JSON line) on a `chat_open` stream, either way.
    Chat {
        line: String,
    },
    /// One SSE message from the kanboard daemon (`board_sse` streams).
    BoardEvent {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        event: Option<String>,
        data: String,
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
    /// UNI-216: phone → host input/quality/acks on a `desktop_open` stream.
    DesktopCmd {
        cmd: crate::desktop::DesktopCmd,
    },
    /// UNI-216: host → phone event on a `desktop_open` stream (a `frame`
    /// event is followed by kind-1 frames with the H.264 access unit).
    DesktopEvent {
        event: crate::desktop::DesktopEvent,
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
            .map_err(ProtoError::from_path_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_list_round_trips() {
        let json = r#"{"type":"hello","proto":"1.0.0","req":{"op":"list","mux":"herdr"}}"#;
        let parsed: Control = serde_json::from_str(json).expect("parse");
        match parsed {
            Control::Hello { proto, req } => {
                assert_eq!(proto.as_str(), "1.0.0");
                match req {
                    Request::List { mux } => assert_eq!(mux, MuxKind::Herdr),
                    _ => panic!("expected List"),
                }
            }
            _ => panic!("expected Hello"),
        }
    }

    #[test]
    fn attach_rejects_out_of_range_cols() {
        let json = r#"{"type":"hello","proto":"1.0.0","req":{"op":"attach","mux":"tmux","session":"s","cols":0,"rows":24,"size_policy":"fit_phone"}}"#;
        let parsed: Result<Control, _> = serde_json::from_str(json);
        assert!(parsed.is_err());
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
