//! Extension UI plumbing (ui-protocol.md §2–§6, §9).
//!
//! [`bridge`] keeps the dialog protocol (request/response, first answer wins,
//! dismissal, handler-deadline withdrawal) and the one-shot effects. [`registry`]
//! is the retained state of §6: every retained producer call (`setStatus`,
//! `setWidget`, `setHeader`, `setFooter`, `openPanel`, `updatePanel`, …) is an
//! owned entry in a per-scope registry that publishes `UiSnapshot`/`UiPatch` with
//! a monotonic `rev`.
//!
//! The session owns one registry per scope (`session`); fronts read it either
//! directly (`_oino/ui_snapshot`, `_oino/ui_resync`) or through the hub
//! (`ClientEvent::Oino(OinoEvent::UiPatch)`).

pub mod bridge;
pub mod registry;

pub use bridge::{
    DismissReason, UiAnswer, UiBridge, UiOutput, UiRequest, UiResponse, UiSink, is_dialog,
    prompt_kind,
};
pub use oino_ui::slot::{
    ActionSource, DismissReason as RegistryDismissReason, RegistryOp, Scope, Slot, UiAction,
    UiCapabilities, UiEntry, UiPatch, UiSnapshot, UiView, UiVisible,
};
pub use registry::{
    EntryRequest, EntryUpdate, ProduceOutcome, PutOutcome, RegistryError, RegistrySink,
    ReservedPanelSpec, UiRegistry, VisibilityChange, clear_reserved_panels, default_order,
    is_producer, register_reserved, reserved, reserved_panels,
};
