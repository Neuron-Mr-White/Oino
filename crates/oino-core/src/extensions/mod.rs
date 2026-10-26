//! Extension runner (pi `core/extensions/runner.ts`) over native extension instances.
//!
//! | Module | Contents |
//! |---|---|
//! | [`api`] | instance contract, registrations, call context |
//! | [`host`] | [`host::HostApi`], [`host::HostOp`]: what extensions can do |
//! | [`runner`] | registries and pi-composition dispatch |
//! | [`tool`] | extension tools as `AgentTool`s |

pub mod api;
pub mod host;
pub mod runner;
pub mod tool;
