//! Wire protocol types and framing for `unipi/app/1`.
//!
//! This crate mirrors `packages/protocol` (zod schemas) on the Rust
//! side: the same `Control`/`Request`/`AppErrorDto` shapes, validated
//! with `serde`, plus the frame codec. See `unipi-app/DESIGN.md` §5.4
//! and §5.5.

pub mod admin;
pub mod blob;
pub mod browser;
pub mod chat;
pub mod board;
pub mod control;
pub mod cron;
pub mod desktop;
pub mod error;
pub mod forward;
pub mod frame;
pub mod fs;
pub mod git;
pub mod paths;
pub mod push;
pub mod sharing;
pub mod version;

pub use admin::{DeviceItem, DevicesListData, HostInfoData, HostTools, PairBeginData, PairExpiredReason, PairStatusData};
pub use blob::{
    BlobGetData, BlobMime, BlobName, BlobPath, BlobPutData, BlobRange, BlobSize, Sha256Hex,
    BLOB_MAX_SIZE,
};
pub use browser::{
    BrowserAgent, BrowserCmd, BrowserEvent, BrowserFrameMeta, BrowserInfo, BrowserOpenData, BrowserProfile,
    BrowserProfileInfo, BrowserProfilesData, BrowserStatusData, BrowserTab, BrowserUrl, BrowserViewport,
};
pub use chat::{
    ChatCloseData, ChatClosed, ChatConfigData, ChatLauncher, ChatNewData, ChatNewTarget,
    CHAT_LAUNCHER_DEFAULT, CHAT_LAUNCHER_MAX,
};
pub use board::{
    check_board_path, BoardHttpData, BoardMethod, BoardPath, BoardProject, BodyLen, ContentType,
    HttpStatus, BOARD_MAX_BODY, BOARD_MAX_PATH,
};
pub use control::{
    AgentSessionRef, AgentStatus, AttachData, ChatBridgeRef, ChatWaiting, AttachTarget, Control, Dimension, HerdrId, HerdrPane,
    HerdrSession, HerdrState, HerdrTab, HerdrWorkspace, HostId, HostName, Lines, ListData,
    MuxAction, MuxKind, NotifyAppDetail, NotifyConfigData, NotifyEndpoint, PairCode, PairData, PairName, PaneAgent, PreviewData, PreviewText, Request,
    SessionId, SessionName, SessionSummary, ShortcutKey, ShortcutMux, ShortcutsData, SizePolicy,
    Title, TmuxId, TmuxPane, TmuxPaneId, TmuxSession, TmuxSessionId, TmuxState, TmuxWindow,
    TmuxWindowId, TreeData,
};
pub use desktop::{
    DesktopCmd, DesktopConsent, DesktopDisplay, DesktopEvent, DesktopFrameMeta, DesktopQuality, DesktopStatusData,
    DesktopViewport, DisplayKind, PointerButton, DESKTOP_FEATURE_DISPLAYS, DESKTOP_FEATURE_PRESETS, DESKTOP_MAX_SIDE,
    DESKTOP_MAX_TEXT,
};
pub use cron::{
    CronAddRequest, CronEditRequest, CronJob, CronJobId, CronNotify, CronOpenSessionData, CronWake, CronJobStatus, CronJobsData, CronLogData, CronName,
    CronParseData, CronPrompt, CronRun, CronRunId, CronRunStartedData, CronRunStatus, CronRunsData, CronRunsLimit,
    CronSettingsData, CronSettingsSetRequest, CronShowData, CronStatusData, CronStoppedData, CRON_NAME_MAX,
    CRON_PARSE_PREVIEW, CRON_PROMPT_MAX, CRON_RUNS_LIMIT_MAX,
};
pub use error::{AppErrorCode, AppErrorDto, PathSegment, ProtoError};
pub use forward::{ForwardPort, ListeningPort, PortsListData};
pub use frame::{DecodedFrame, Decoder, FrameError, FrameKind, FRAME_HEADER_LEN, MAX_FRAME};
pub use fs::{
    EntryKind, FsConflictDetail, FsEntry, FsListData, FsPath, FsReadData, FsRootsData,
    FsSearchData, FsSearchResult, FsStatData, FsWriteData, SearchKind, SearchQuery,
    FS_LIST_CAP, FS_READ_CAP, FS_SEARCH_CAP,
};
pub use git::{
    GitArea, GitBranch, GitBranchesData, GitCmd, GitCommitData, GitCommitInfo, GitDiffData,
    GitFileChange, GitLogData, GitMessage, GitPath, GitRefName, GitRepoData, GitShowData,
    GitShortstatData, GitShowFile, GitStatusData, GitSyncData, GIT_DIFF_CAP, GIT_LOG_MAX, GIT_PATHS_MAX,
    GIT_SHORTSTAT_UNTRACKED_MAX,
};
pub use push::{PushId, PushItem, PushListData, PushOutcome, PushWait, PUSH_WAIT_MAX};
pub use sharing::{
    FeatureKind, FeatureLeaveData, FeatureLeaveScope, FeatureNote, FeatureRequestItem, FeatureRequestStatus,
    FeatureRequestsData, FeatureScope, FEATURE_NOTE_MAX, FEATURE_PORTS_MAX,
};
pub use version::{is_strict_semver, Semver, PROTO_VERSION};
