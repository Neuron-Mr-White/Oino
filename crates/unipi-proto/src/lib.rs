//! Wire protocol types and framing for `unipi/oino/1` (proto v2).
//!
//! This crate mirrors `packages/protocol` (zod schemas) on the Rust
//! side: the same `Control`/`Request`/`AppErrorDto` shapes, validated
//! with `serde`, plus the frame codec (host.md §7). The v2 skeleton
//! drops the mux/board/desktop/chat layer; session/term/settings wire
//! types land with their work packages.

pub mod admin;
pub mod blob;
pub mod board;
pub mod browser;
pub mod control;
pub mod cron;
pub mod error;
pub mod forward;
pub mod frame;
pub mod fs;
pub mod git;
pub mod paths;
pub mod push;
pub mod session;
pub mod settings;
pub mod sharing;
pub mod term;
pub mod version;

pub use admin::{
    DeviceItem, DevicesListData, HostInfoData, HostTools, PairBeginData, PairExpiredReason,
    PairStatusData,
};
pub use blob::{
    BlobGetData, BlobMime, BlobName, BlobPath, BlobPutData, BlobRange, BlobSize, Sha256Hex,
    BLOB_MAX_SIZE,
};
pub use board::{
    BoardCliData, BoardErrorKind, BoardSlug, BoardWatchData, BOARD_ARGS_MAX, BOARD_ARG_MAX,
    BOARD_SLUG_MAX,
};
pub use browser::{
    BrowserAgent, BrowserCmd, BrowserEvent, BrowserFrameMeta, BrowserInfo, BrowserOpenData,
    BrowserProfile, BrowserProfileInfo, BrowserProfilesData, BrowserStatusData, BrowserTab,
    BrowserUrl, BrowserViewport,
};
pub use control::{
    Control, Dimension, HostId, HostName, NotifyAppDetail, NotifyConfigData, NotifyEndpoint,
    PairCode, PairData, PairName, Request, Title,
};
pub use cron::{
    CronAddRequest, CronEditRequest, CronJob, CronJobId, CronJobStatus, CronJobsData, CronLogData,
    CronName, CronNotify, CronParseData, CronPrompt, CronRun, CronRunId, CronRunStartedData,
    CronRunStatus, CronRunsData, CronRunsLimit, CronSettingsData, CronSettingsSetRequest,
    CronShowData, CronStatusData, CronStoppedData, CronWake, CRON_NAME_MAX, CRON_PARSE_PREVIEW,
    CRON_PROMPT_MAX, CRON_RUNS_LIMIT_MAX,
};
pub use error::{AppErrorCode, AppErrorDto, PathSegment, ProtoError};
pub use forward::{ForwardPort, ListeningPort, PortsListData};
pub use frame::{DecodedFrame, Decoder, FrameError, FrameKind, FRAME_HEADER_LEN, MAX_FRAME};
pub use fs::{
    EntryKind, FsConflictDetail, FsEntry, FsListData, FsPath, FsReadData, FsRootsData,
    FsSearchData, FsSearchResult, FsStatData, FsWriteData, SearchKind, SearchQuery, FS_LIST_CAP,
    FS_READ_CAP, FS_SEARCH_CAP,
};
pub use git::{
    GitArea, GitBranch, GitBranchesData, GitCmd, GitCommitData, GitCommitInfo, GitDiffData,
    GitFileChange, GitLogData, GitMessage, GitPath, GitRefName, GitRepoData, GitShortstatData,
    GitShowData, GitShowFile, GitStatusData, GitSyncData, GitTurnFile, GitTurnFilesData,
    GIT_DIFF_CAP, GIT_LOG_MAX, GIT_PATHS_MAX, GIT_SHORTSTAT_UNTRACKED_MAX,
};
pub use push::{PushId, PushItem, PushListData, PushOutcome, PushWait, PUSH_WAIT_MAX};
pub use session::{
    HubEvent, NoticeLevel, RunState, SessionCard, SessionEventDto, SessionEvents, SessionId,
    SessionName, SessionReply, SessionSnapshot, SessionTarget, SessionsListData, HOST_EVENT_KINDS,
    SESSION_ID_MAX, SESSION_NAME_MAX, SESSION_TARGET_MAX,
};
pub use settings::{
    HostSettingsKey, HostSettingsSetRequest, RelaySetting, HOST_SETTINGS_BLOCKS,
    HOST_SETTINGS_KEYS, SECRET_MASK,
};
pub use sharing::{
    FeatureKind, FeatureLeaveData, FeatureLeaveScope, FeatureNote, FeatureRequestItem,
    FeatureRequestStatus, FeatureRequestsData, FeatureScope, FEATURE_NOTE_MAX, FEATURE_PORTS_MAX,
};
pub use term::{TermAttachData, TermId, TermInfo, TermShell, TERM_ID_MAX, TERM_SHELL_MAX};
pub use version::{is_strict_semver, Semver, PROTO_VERSION};
