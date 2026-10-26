//! Construction of the built-in tools (pi `tools/index.ts`: `createTool`, `createAllTools`,
//! `createCodingTools`, `createReadOnlyTools`).

use std::sync::Arc;

use oino_agent::AgentTool;

use crate::bash::ShellTool;
use crate::config::ToolConfig;
use crate::edit::EditTool;
use crate::find::FindTool;
use crate::grep::GrepTool;
use crate::info::{
    BuiltinToolInfo, BuiltinToolName, DEFAULT_ACTIVE_TOOLS, READ_ONLY_TOOLS, builtin_tool_info,
};
use crate::ls::LsTool;
use crate::read::ReadTool;
use crate::write::WriteTool;

/// pi `createTool(name)`.
#[must_use]
pub fn create_tool(name: BuiltinToolName, config: ToolConfig) -> Arc<dyn AgentTool> {
    match name {
        BuiltinToolName::Read => Arc::new(ReadTool::new(config)),
        BuiltinToolName::Bash => Arc::new(ShellTool::bash(config)),
        BuiltinToolName::Powershell => Arc::new(ShellTool::powershell(config)),
        BuiltinToolName::Edit => Arc::new(EditTool::new(config)),
        BuiltinToolName::Write => Arc::new(WriteTool::new(config)),
        BuiltinToolName::Grep => Arc::new(GrepTool::new(config)),
        BuiltinToolName::Find => Arc::new(FindTool::new(config)),
        BuiltinToolName::Ls => Arc::new(LsTool::new(config)),
    }
}

fn create_all(names: &[BuiltinToolName], config: &ToolConfig) -> Vec<Arc<dyn AgentTool>> {
    names
        .iter()
        .map(|name| create_tool(*name, config.clone()))
        .collect()
}

/// pi `createAllTools`: every built-in available on this platform (`powershell` only on
/// Windows), in pi order.
#[must_use]
pub fn builtin_tools(config: &ToolConfig) -> Vec<Arc<dyn AgentTool>> {
    create_all(&BuiltinToolName::all(), config)
}

/// pi `createCodingTools`: the default active set ([`DEFAULT_ACTIVE_TOOLS`]).
#[must_use]
pub fn coding_tools(config: &ToolConfig) -> Vec<Arc<dyn AgentTool>> {
    create_all(&DEFAULT_ACTIVE_TOOLS, config)
}

/// pi `createReadOnlyTools`.
#[must_use]
pub fn read_only_tools(config: &ToolConfig) -> Vec<Arc<dyn AgentTool>> {
    create_all(&READ_ONLY_TOOLS, config)
}

/// A built-in by model-facing name (`None` for unknown names or `powershell` off Windows).
#[must_use]
pub fn tool_by_name(name: &str, config: ToolConfig) -> Option<Arc<dyn AgentTool>> {
    BuiltinToolName::parse(name).map(|name| create_tool(name, config))
}

/// Prompt metadata of every available built-in, in pi order.
#[must_use]
pub fn builtin_tool_infos(config: &ToolConfig) -> Vec<BuiltinToolInfo> {
    BuiltinToolName::all()
        .into_iter()
        .map(|name| builtin_tool_info(name, config.shell.expose_session_environment))
        .collect()
}
