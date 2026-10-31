//! The host's session hub, cards, crash isolation and safe mode (spec-host §4.2, §4.4;
//! WP-H3), the per-client session streams over it (§7.3–7.5; WP-H4), plus the host services
//! registered with the wasm host (§4.2, ext-abi §3).

pub mod bound;
pub mod card;
pub mod crash;
pub mod hub;
pub mod meta;
pub mod services;
pub mod stream;

pub use card::{HubState, ProjectGroup, SessionCard};
pub use hub::{
    takeover_from, HostHubOptions, HubError, HubEvent, HubSession, NoticeLevel, SessionHub,
};
pub use meta::{FocusRegistry, MetaStore, SessionMetaFlags};
pub use stream::{Feed, SessionService, Subscription};
