//! Extension runner (pi `core/extensions/runner.ts`) over native extension instances.
//!
//! | Module | Contents |
//! |---|---|
//! | [`api`] | instance contract, registrations, call context |
//! | [`deadline`] | handler deadlines that pause while a dialog is open |
//! | [`host`] | [`host::HostApi`], [`host::HostOp`]: what extensions can do |
//! | [`runner`] | registries and pi-composition dispatch |
//! | [`settings`] | extension settings registry: schema, layered values, secret fields, writes, change events |
//! | [`tool`] | extension tools as `AgentTool`s |

pub mod api;
pub mod deadline;
pub mod host;
pub mod runner;
pub mod settings;
pub mod tool;
