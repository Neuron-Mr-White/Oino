//! `oino-tools`: the built-in tools — `read`, `write`, `edit`, `bash`, `grep`, `find`,
//! `ls` and (Windows) `powershell` — with fixed names, schemas, descriptions, prompt text,
//! error strings, truncation footers and `details` shapes.
//!
//! Tools implement [`oino_agent::AgentTool`]; `oino-core` builds them from a [`ToolConfig`]
//! (cwd, agent dir, [`ExecutionEnv`], live [`ToolContext`], image and shell options) and uses
//! [`BuiltinToolInfo`] for the system prompt. Temp files go to `<agent>/tmp/oino-bash-*.log`;
//! the shell environment is D12.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`registry`] | [`builtin_tools`], [`coding_tools`], [`read_only_tools`], [`create_tool`], [`tool_by_name`] |
//! | [`info`] | [`BuiltinToolName`], [`BuiltinToolInfo`], [`DEFAULT_ACTIVE_TOOLS`] |
//! | [`config`] | [`ToolConfig`], [`ToolContext`], [`SessionEnv`], [`ImageOptions`], [`ShellOptions`] |
//! | [`mod@env`] | [`ExecutionEnv`], [`LocalEnv`], [`SpawnedProcess`], [`FsError`] |
//! | [`read`] | [`ReadTool`], [`read::block_images`] |
//! | [`mod@write`] | [`WriteTool`] |
//! | [`edit`] | [`EditTool`], [`edit::prepare_edit_arguments`] |
//! | [`bash`] | [`ShellTool`], [`BashDetachRegistry`], [`run_shell_command`], [`bash::resolve_shell_config`] |
//! | [`grep`], [`find`], [`ls`] | [`GrepTool`], [`FindTool`], [`LsTool`] |
//! | [`truncate`] | [`truncate::truncate_head`], [`truncate::truncate_tail`], [`truncate::truncate_line`], [`truncate::format_size`] |
//! | [`path_utils`] | [`path_utils::expand_path`], [`path_utils::resolve_to_cwd`], [`path_utils::resolve_read_path`] |
//! | [`mutation_queue`] | [`FileMutationQueue`] |
//! | [`image`] | [`image::process_image`], [`image::detect_supported_image_mime_type`] |

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]
#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

mod args;
mod canonical;
pub mod apply_patch;
pub mod legacy;
pub mod bash;
mod collate;
pub mod config;
pub mod edit;
pub mod env;
pub mod find;
pub mod grep;
pub mod image;
pub mod info;
mod js;
pub mod ls;
pub mod mutation_queue;
mod node_path;
pub mod path_utils;
pub mod read;
pub mod registry;
pub mod truncate;
pub mod write;

pub use apply_patch::patch_paths;
pub use bash::{
    BashDetachRegistry, BashOutcome, DetachedProcess, ShellRunOptions, ShellTool, run_shell_command,
};
pub use config::{
    ImageOptions, ModelImageInfo, NoContext, SessionEnv, ShellOptions, ToolConfig, ToolContext,
};
pub use canonical::canonical_args;
pub use edit::EditTool;
pub use env::{ExecutionEnv, FsError, LocalEnv, SpawnedProcess};
pub use find::FindTool;
pub use grep::GrepTool;
pub use info::{
    BuiltinToolInfo, BuiltinToolName, DEFAULT_ACTIVE_TOOLS, READ_ONLY_TOOLS, builtin_tool_info,
};
pub use js::{CodemodeTool, ToolSearchTool};
pub use ls::LsTool;
pub use mutation_queue::FileMutationQueue;
pub use path_utils::resolve_to_cwd as canonical_path;
pub use read::ReadTool;
pub use registry::{
    builtin_tool_infos, builtin_tools, coding_tools, create_tool, read_only_tools, tool_by_name,
};
pub use write::WriteTool;

/// The abort error text.
pub const OPERATION_ABORTED: &str = "Operation aborted";

fn aborted() -> oino_agent::ToolError {
    oino_agent::ToolError::new(OPERATION_ABORTED)
}

/// A JSON number as JavaScript serialises it (whole numbers without a fraction).
fn js_number(value: f64) -> oino_ai::JsonValue {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "whole and within the safe integer range"
        )]
        let whole = value as i64;
        serde_json::json!(whole)
    } else {
        serde_json::json!(value)
    }
}
