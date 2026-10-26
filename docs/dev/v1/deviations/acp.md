# Deviations: acp

Owner: `acp` (`crates/oino-acp`). Format per README §5: section, change, reason, impact.
Reference: modes.md §6, research-acp.md, `agent-client-protocol =3.5.0` (schema 1.12.0).

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §6.2 `session/fork` | The fork is a new session file copying the source's whole current branch (`SessionSource::Fork`), not `fork(entryId = last user message, Before)`. | Fork-before drops the last turn and its text (meant for "edit message" flows); `AgentSessionRuntime::fork` replaces the runtime's session while built-in tools stay bound to the old one (modes deviation 19). Clients use `session/fork` to branch a thread. | Forks keep the last turn. |
| 2 | modes §6.2 `session/new` | `mcpServers` are not published on bus topic `oino:acp:mcp-servers`; a non-empty list only produces the `notice` "MCP servers ignored: no MCP extension enabled" (also on `session/load`). | Core has no API for a frontend to publish bus data. | unipi `mcp` cannot pick up client servers yet (see requests). |
| 3 | modes §6.2 `session/prompt` | `stopReason: "refusal"` is never produced. A provider error ends with `end_turn` after a `notice{severity: error, title: errorMessage}`. | No provider refusal classification exists in `oino-ai`/core. | Refusals look like errors. |
| 4 | modes §6.2 | `session/new`/`load` return no `modes` (`SessionModeState`); `session/set_mode` is still accepted (mode id = thinking level) and `current_mode_update` is never sent. | `configOptions` already carry the thinking level; returning both shows two selectors. | pi-acp-style clients that only read `modes` see no thinking selector. |
| 5 | modes §6.1 | `sessionCapabilities.fork: {}` is advertised too (SDK feature `unstable_session_fork`). | `session/fork` is implemented. | none. |
| 6 | modes §6.2 `_oino/*` | ACP extension methods are not session-routed, so every `_oino/<command>` takes `sessionId` in its params (removed before decoding). `_oino/rpc {sessionId, command}` runs any pi or `_oino/*` RPC command (`steer`, `follow_up`, `get_tree`, …). The SDK delivers extension methods without their leading `_`; the adapter restores it. | ACP v1 has no per-session routing for extension methods. | Clients add `sessionId`. |
| 7 | modes §6.3 tool kinds | Kinds are mapped by built-in tool name (`read`→read, `write`/`edit`→edit, `bash`/`powershell`→execute, `grep`/`find`/`ls`→search); every extension tool is `other`. | Core `ToolInfo` has no `kind` yet (runtime §7.4). | Extension tools render generically. |
| 8 | modes §6.3 `tool_execution_update` | When `bash` runs in a client terminal, `{type: terminal, terminalId}` is attached as soon as `terminal/create` returns and text partial results are not sent (the terminal shows the output); the final update keeps the terminal content, `rawOutput` holds the tool result (truncated output). | The client renders the live terminal. | none. |
| 9 | modes §6.3 commands | `available_commands_update` lists adapter built-ins `/compact [instructions]`, `/name <name>`, `/export [path]` (HTML), `/session` (stats), which the adapter runs through the session API (modes §1.3) and answers as agent text, followed by the session's extension commands, prompt templates and skills (`input.hint: "arguments"`). Other pi frontend built-ins map to ACP instead (`/model`, `/thinking` → config options; `/new`, `/resume`, `/fork` → session methods) and are not listed. | ACP has no built-in command surface. | none. |
| 10 | modes §6.3 (addition) | Plans: the fire-and-forget UI method `setPlan {entries: [{content, priority, status}]}` becomes a `plan` update. | Core has no plan concept; extensions (unipi workflow/todo) need a way to publish ACP plans. | ui-protocol should adopt `setPlan` (see requests); other frontends receive it as an unknown fire-and-forget request. |
| 11 | modes §6.4 dialogs | Elicitation forms have one property `value`: `select` (string enum), `input`/`editor` (string, `default` = prefill), `confirm` (boolean), `multiSelect` (array of enum). `form`, `custom`, `clipboardReadImage`, `editExternally` resolve `cancelled`; `approval.feedback` is not collected. `extension_ui_dismiss` is forwarded as `_oino/extension_ui`, but an open client request is not cancelled (`$/cancel_request` is not sent). | Restricted ACP form schemas; FormQuestion trees need a multi-step mapping. | unipi ask_user forms are unavailable over ACP. |
| 12 | modes §6.4 fs delegation | Reads go through `fs/read_text_file` only for text files (local sniff of the first 8 KiB: UTF-8 without NUL); binary files (images) are read locally. Client failures fall back to the local file system (Node-style errors). Writes of non-UTF-8 data are local. `access`/`stat`/`read_dir`/`realpath`/`mkdir_p` are always local. Client terminals are used only when no stdin is piped; `terminal/create.env` carries only variables that differ from the agent's environment; output is polled every 100 ms with `outputByteLimit` 16 MiB. Diff snapshots read the local file. | ACP fs is text-only; the agent runs on the same machine as the client. | none. |
| 13 | modes §6.3 `usage_update` | Sent after `turn_end`, `compaction_end` and `agent_settled` only when the context token count is known (it is `null` right after compaction). | `used` is required. | none. |
| 14 | modes §6.2 | The ACP session id is the Oino session id of the runtime's first session and stays fixed if the session is later replaced through `_oino/rpc` (`new_session`, `switch_session`, …). `session/list` titles fall back to the first line of the first message (≤ 80 chars). | ACP ids must stay stable for the client. | none. |
| 15 | modes §6.3 | Extension UI raised by `session_start` handlers before the adapter subscribes is not forwarded (recover with `_oino/rpc {command: {type: "_oino/ui_snapshot"}}`). | `AgentSessionRuntime::create` runs `session_start` before a backend exists (same as modes deviation 7). | none in practice. |
| 16 | modes §6.5 | Acceptance tests are scripted JSON-RPC transcripts with assertions (not byte-golden files) plus an interop test with the SDK's typed `Client` instead of `agent-client-protocol-test`. Zed interop is manual (not yet run: `--mode acp` is not wired, see requests). | Event ids/timestamps vary; the typed client is the strict decoder. | none. |

## Requests for other work

- **modes**: wire `--mode acp` in `crates/oino` (owned by modes): drop `AppMode::Acp` from the
  unavailable list in `preflight`, and before `startup::start` dispatch to a new
  `modes::acp::run`:

  ```rust
  // crates/oino/src/modes/acp.rs
  pub async fn run(args: &Args, env: &Environment) -> i32 {
      let (agent_dir, extensions) = (env.agent_dir.clone(), args.extensions.clone().unwrap_or_default());
      let flags = ResourceFlags { /* same as startup::start */ ..ResourceFlags::default() };
      let (values, approve) = (args.unknown_flags.clone(), args.project_trust_override);
      let factory = oino_acp::CoreSessionFactory::new(
          move |cwd| ServicesOptions {
              cwd: cwd.to_path_buf(), agent_dir: agent_dir.clone(), docs_root: agent_dir.clone(),
              mode: ExtensionMode::Acp, has_ui: true, flags: flags.clone(),
              extension_flag_values: values.clone(),
              loader: Arc::new(CliLoader::new(extensions.clone())),
              approve, models: None, settings: None,
          },
          oino_acp::SessionDefaults {
              session_dir: args.session_dir.as_ref().map(PathBuf::from),
              model: None, // or resolve `--model` once against the first Services
              thinking_level: args.thinking,
              tools: tool_selection(args),
              exclude_tools: args.exclude_tools.clone().unwrap_or_default(),
          },
      );
      let info = oino_acp::AgentInfo { version: crate::VERSION.to_owned(), ..Default::default() };
      match oino_acp::serve_stdio(Arc::new(factory), info, env.cwd.clone()).await {
          Ok(()) => 0,
          Err(error) => { crate::eprintln_line(&format!("Error: {error}")); 1 }
      }
  }
  ```

  Logs must stay on stderr (stdout carries ACP). Add `oino-acp = { path = "../oino-acp" }`.
- **core-runtime**: (a) a frontend API to publish bus data (`oino:acp:mcp-servers`, #2);
  (b) `ToolInfo.kind` from tool specs (#7); (c) optionally a refusal classification on
  assistant errors (#3).
- **ui-protocol / unipi-context (workflow)**: adopt fire-and-forget `setPlan` (#10).
- **integration**: root `Cargo.toml` member `crates/oino-acp` was appended by this item.
