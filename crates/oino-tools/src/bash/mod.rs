//! Shell tools and reusable shell execution (pi `tools/bash.ts`, `tools/powershell.ts`,
//! `tools/output-accumulator.ts`, `core/bash-executor.ts`, `utils/shell.ts`).

mod accumulator;
mod detach;
mod exec;
mod executor;
mod shell;
mod tool;

pub use detach::{BashDetachRegistry, DetachedProcess};
pub use exec::resolve_timeout;
pub use executor::{BashOutcome, ShellRunError, ShellRunOptions, run_shell_command};
pub use shell::{
    CommandTransport, POWERSHELL_ARGS, ShellConfig, resolve_powershell_config,
    resolve_shell_config, sanitize_binary_output, shell_env, split_incomplete_ansi_suffix,
    strip_ansi, tool_env,
};
pub use tool::{BASH_UPDATE_THROTTLE, ShellTool};

/// `<agent_dir>/tmp`, where truncated output is saved (pi uses the OS temp directory).
#[must_use]
pub fn temp_dir(agent_dir: &str) -> String {
    crate::node_path::join(agent_dir, "tmp")
}
