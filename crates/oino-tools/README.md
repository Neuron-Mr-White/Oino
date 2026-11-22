<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-tools

The built-in tools for Oino agents: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls` and
(Windows) `powershell`.

Tools implement `oino_agent::AgentTool`; `oino-core` builds them from a `ToolConfig` (cwd,
agent dir, execution environment, live tool context, image and shell options) and uses
`BuiltinToolInfo` for the system prompt.

* `registry` / `info` — construction and static metadata (names, schemas, prompt text)
* `bash` — shell tools, detach registry, output accumulation, truncation-safe logs
* `edit` — argument preparation, replacement application, jsdiff-style line diffs
* `truncate` — head/tail/line truncation with size formatting
* `image`, `path_utils`, `node_path` — image processing and lexical path handling
