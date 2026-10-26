# Extension API: WIT `oino:extension@1.0.0`, host, manifest, SDK, bundling

Status: binding. Owner: `wit-v1` (`wit/`, `crates/oino-extension-api`, `crates/oino-wasm-host`,
`extensions/sdk/oino-extension*`, `extensions/fixtures`). Consumers: `core-runtime` (implements
`HostServices`), `providers-v1`, all `unipi-*`.
Inputs: audit-pi §9 (pi `ExtensionAPI`), audit-unipi §4.1 (capabilities C1–C29).

## 1. Principles

1. **Every pi `ExtensionAPI`/`ExtensionContext` capability and every unipi capability C1–C29
   is reachable** from a guest, gated by manifest permissions (§7.3). §9 is the coverage table.
2. **JSON for pi data, typed WIT for capabilities** (D5). `type json = string` always holds
   canonical JSON of an `oino-ai` / `oino-session` / `oino-protocol` serde type named in the
   doc comment. The guest SDK decodes it with the same crates.
3. **Async everywhere it can wait** (D6): component-model `async func`; the host runs guest
   calls with wasmtime's concurrent call support so several guest tasks can be in flight per
   instance. Nothing in the host serialises calls per extension.
4. **No raw keys, no raw terminals.** Guests get credentialed requests (`credential-for`,
   host HTTP with `auth`), never `auth.json`; guests get editor *intents*, never terminal bytes.
5. One component instance per extension per process; per-session state is keyed by
   `call-context.session-id` (one Oino process may host several sessions: `serve`, desktop).

## 2. Package layout

```text
wit/oino-extension/
  types.wit      host-info.wit  log.wit       registry.wit  session.wit  control.wit
  models.wit     ui.wit         agents.wit    process.wit   tasks.wit    http.wit
  fs.wit         timers.wit     kv.wit        secrets.wit   bus.wit      services.wit
  guest.wit      provider.wit   provider-host.wit  worlds.wit
wit/deps/        wasi:cli, wasi:clocks, wasi:random, wasi:filesystem, wasi:io  (0.2.x)
```

`wasm-tools component wit wit/oino-extension` must validate in CI. The text below is the
complete package; files are split by interface as listed.

## 3. WIT text

```wit
package oino:extension@1.0.0;

// ───────────────────────────── types.wit ─────────────────────────────
interface types {
    /// Canonical JSON text of a serde type named in the doc comment of each use.
    type json = string;
    /// Extension id, `^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$`.
    type extension-id = string;
    /// Session entry id (8-hex or legacy string).
    type entry-id = string;

    enum log-level { trace, debug, info, warn, error }
    enum mode { tui, rpc, json, print, acp, desktop, mobile }
    enum call-kind { init, event, tool, command, shortcut, service, nudge, render, route, provider }

    record call-context {
        session-id: string,
        mode: mode,
        has-ui: bool,
        cwd: string,
        is-child: bool,
        child-role: option<string>,
        kind: call-kind,
        /// Set while serving a tool call; enables `control.execute-tool` and nested ids.
        tool-call-id: option<string>,
    }

    variant host-error {
        denied(string),          // permission missing (names the permission)
        stale-context,           // session replaced since this call started (pi assertActive)
        command-context-required,
        invalid-argument(string),
        not-found(string),
        timeout,
        cancelled,
        io(string),
        failed(string),
    }

    /// Cooperative cancellation for long calls (tools, commands, dialogs).
    resource cancel-token {
        is-cancelled: func() -> bool;
        /// Resolves when cancelled; never resolves otherwise.
        wait: async func();
    }
}

// ───────────────────────────── log.wit ──────────────────────────────
interface log {
    use types.{log-level};
    log: func(level: log-level, message: string);
}

// ─────────────────────────── host-info.wit ──────────────────────────
interface host-info {
    use types.{mode, host-error};
    record info {
        oino-version: string,
        api-version: string,          // "1.0.0"
        platform: string,             // "linux" | "macos" | "windows" | "android" | "ios"
        arch: string,
        mode: mode,
        has-ui: bool,
        frontends: list<string>,      // attached frontends: "tui","desktop","mobile","rpc","acp"
        extension-version: string,
    }
    record workspace {
        id: string,                   // D10 wsid
        root: string,                 // canonical project root
        git-root: option<string>,
        git-branch: option<string>,
        cwd: string,
    }
    get: func() -> info;
    workspace: func() -> workspace;
    /// Requires permission env:[name]; returns none when unset.
    env-get: func(name: string) -> result<option<string>, host-error>;
    frontend-focused: func() -> bool;
}

// ─────────────────────────── registry.wit ───────────────────────────
interface registry {
    use types.{json, host-error};

    enum event-kind {
        // pi 1.4.0 extension events (wire name = kebab → snake, e.g. session-start → session_start)
        project-trust, resources-discover, mcp-servers-change,
        session-start, session-info-changed, session-before-switch, session-before-fork,
        session-before-compact, session-compact, session-compact-failed, session-shutdown,
        session-before-tree, session-tree,
        context, context-with-system, cache-warming-decision,
        before-provider-headers, before-provider-request, after-provider-response, provider-stream-event,
        before-agent-start, agent-start, agent-end, agent-before-settle, agent-settled,
        ui-prompt-start, ui-prompt-end,
        turn-start, turn-end, message-start, message-update, message-end,
        tool-execution-start, tool-execution-update, tool-execution-end,
        model-select, thinking-level-select, tool-call, tool-result, user-bash, input,
        // Oino host events (wire names prefixed "oino_")
        oino-timer, oino-bus, oino-ui-action, oino-editor, oino-settings-changed,
        oino-process, oino-agent, oino-task,
    }

    enum exposure { direct, model-only, deferred, hidden }
    enum execution-mode { parallel, sequential }
    enum replay { never, safe }
    /// ACP tool kind (research-acp §7).
    enum tool-kind { read, edit, delete, move, search, execute, think, fetch, switch-mode, other }
    record tool-namespace { name: string, aliases: list<string>, description: option<string>, instructions: option<string> }
    record tool-annotations { read-only: option<bool>, destructive: option<bool>, idempotent: option<bool>, open-world: option<bool> }
    variant reaches-tools { none, all, names(list<string>), dynamic }   // dynamic ⇒ guest export reaches-tools
    record tool-spec {
        name: string,
        label: string,
        description: string,
        /// JSON Schema object (draft 2020-12).
        parameters: json,
        output-schema: option<json>,
        prompt-snippet: option<string>,
        prompt-guidelines: list<string>,
        exposure: exposure,
        default-active: bool,
        execution-mode: execution-mode,
        replay: option<replay>,
        kind: tool-kind,
        namespace: option<tool-namespace>,
        annotations: option<tool-annotations>,
        reaches-tools: reaches-tools,
        /// Guest implements `prepare-arguments` for this tool.
        prepare-arguments: bool,
        /// Guest implements `prepare-loadout` for this tool.
        prepare-loadout: bool,
        /// JSON of oino_protocol::ui::ToolRenderHints (ui-protocol §6.4), or none.
        render: option<json>,
    }
    record command-spec {
        name: string,
        description: string,
        argument-hint: option<string>,
        /// Guest implements `complete-command` for this command.
        completions: bool,
        /// Listed in get_commands only when false.
        hidden: bool,
    }
    record shortcut-spec { id: string, default-keys: list<string>, description: string }
    enum flag-type { boolean, %string }
    record flag-spec { name: string, %type: flag-type, default-value: option<json>, description: string }
    enum renderer-target { custom-message, custom-entry, tool }
    record renderer-spec { target: renderer-target, key: string }   // customType or tool name
    record arbiter-roles { nudge-provider: bool, wait-source: bool }

    record registration {
        events: list<event-kind>,
        tools: list<tool-spec>,
        commands: list<command-spec>,
        shortcuts: list<shortcut-spec>,
        flags: list<flag-spec>,
        renderers: list<renderer-spec>,
        services: list<string>,
        arbiter: arbiter-roles,
        /// JSON of Vec<oino_protocol::ext::SettingsNamespace> (schema + defaults), ui-protocol §6.6.
        settings: list<json>,
    }

    // Dynamic (re)registration at any time after init.
    subscribe: func(events: list<event-kind>);
    unsubscribe: func(events: list<event-kind>);
    register-tool: func(spec: tool-spec) -> result<_, host-error>;      // same name from same ext = replace
    unregister-tool: func(name: string);
    register-command: func(spec: command-spec) -> result<string, host-error>;  // returns invocation name
    unregister-command: func(name: string);
    register-shortcut: func(spec: shortcut-spec) -> result<_, host-error>;
    unregister-shortcut: func(id: string);
    register-flag: func(spec: flag-spec) -> result<_, host-error>;
    get-flag: func(name: string) -> option<json>;
    register-renderer: func(spec: renderer-spec);
    register-service: func(name: string) -> result<_, host-error>;
    set-arbiter-roles: func(roles: arbiter-roles);
    /// JSON of oino_protocol::ext::SettingsNamespace.
    register-settings: func(namespace: json) -> result<_, host-error>;
    /// Provider registration (pi registerProvider): JSON of oino_protocol::ext::ProviderRegistration.
    /// Only from components exporting `provider`, or a config-only override (baseUrl/headers/models) from any extension.
    register-provider: func(config: json) -> result<_, host-error>;
    unregister-provider: func(provider-id: string);
    /// JSON of oino_protocol::ext::VirtualModelDefinition (route implemented by guest export `route-model`).
    register-virtual-model: func(definition: json) -> result<_, host-error>;
    unregister-virtual-model: func(provider: string, id: string);
}

// ─────────────────────────── session.wit ────────────────────────────
interface session {
    use types.{json, entry-id, host-error};

    enum delivery { steer, follow-up, next-turn }
    record provenance { source: string, title: option<string>, synopsis: option<string>, severity: option<string> }
    record send-options { trigger-turn: bool, deliver-as: option<delivery>, provenance: option<provenance> }
    record user-message-options { deliver-as: option<delivery>, expand-prompt-templates: bool, provenance: option<provenance> }
    enum sessions-scope { cwd, all }

    // Reads (pi ReadonlySessionManager). JSON = oino_session types.
    header: func() -> json;                                   // SessionHeader
    session-id: func() -> string;
    session-file: func() -> option<string>;
    leaf-id: func() -> option<entry-id>;
    entry: func(id: entry-id) -> option<json>;                // SessionEntry
    entries: func() -> json;                                  // Vec<SessionEntry>, file order, all branches
    branch: func(from: option<entry-id>) -> json;             // Vec<SessionEntry> root→from
    context-entries: func() -> json;                          // build_context_entries
    projection: func() -> json;                               // SessionProjection
    tree: func() -> json;                                     // Vec<SessionTreeNode>
    label: func(id: entry-id) -> option<string>;
    session-name: func() -> option<string>;

    // Writes
    set-session-name: func(name: option<string>) -> result<_, host-error>;
    set-label: func(id: entry-id, label: option<string>) -> result<_, host-error>;
    /// pi appendEntry: `custom` entry, emits entry_appended. data = JSON value or none.
    append-entry: func(custom-type: string, data: option<json>) -> result<entry-id, host-error>;
    /// pi sendMessage. message = {customType, content, display, details?}.
    send-message: async func(message: json, options: send-options) -> result<_, host-error>;
    /// pi sendUserMessage. content = UserContentField.
    send-user-message: async func(content: json, options: user-message-options) -> result<_, host-error>;

    // Sessions index (C7); permission sessions.readAll.
    sessions-list: async func(scope: sessions-scope) -> result<json, host-error>;   // Vec<SessionInfo>
    session-read: async func(path: string) -> result<json, host-error>;             // {header, entries}
}

// ─────────────────────────── control.wit ────────────────────────────
interface control {
    use types.{json, host-error, cancel-token};

    // Model / thinking (pi setModel, get/setThinkingLevel; permission sessionControl for setters)
    model: func() -> option<json>;                            // Model
    routed-model: func() -> option<json>;
    set-model: func(provider: string, id: string) -> result<bool, host-error>;  // false when no credentials
    thinking-level: func() -> string;
    set-thinking-level: func(level: string) -> result<string, host-error>;     // returns clamped
    models-list: func() -> json;                              // Vec<Model> available (with credentials)
    scoped-models: func() -> json;

    // Run state
    run-state: func() -> json;                                // oino_protocol::events::RunState
    is-idle: func() -> bool;
    has-pending-messages: func() -> bool;
    context-usage: func() -> json;                            // ContextUsage
    session-stats: func() -> json;                            // SessionStats
    system-prompt: func() -> string;
    is-project-trusted: func() -> bool;
    get-settings: func() -> json;                             // pi getSettings(): merged settings copy

    // Actions
    abort: func();
    compact: async func(custom-instructions: option<string>) -> result<json, host-error>;  // CompactionResult
    shutdown: func();

    // Command-context-only (else command-context-required)
    system-prompt-options: func() -> result<json, host-error>;
    wait-for-idle: async func() -> result<_, host-error>;
    new-session: async func(parent-session: option<string>) -> result<bool, host-error>;   // cancelled?
    fork: async func(entry-id: string, at: bool) -> result<json, host-error>;              // {cancelled, selectedText?}
    switch-session: async func(path: string) -> result<bool, host-error>;
    navigate-tree: async func(target-id: string, options: json) -> result<json, host-error>; // NavigateTreeOptions → NavigateTreeResult
    reload: async func() -> result<_, host-error>;

    // Tools and commands
    active-tools: func() -> list<string>;
    set-active-tools: func(names: list<string>) -> result<_, host-error>;
    all-tools: func() -> json;                                // Vec<ToolInfo>
    commands-list: func() -> json;                            // Vec<SlashCommandInfo>
    /// Run any command (cross-extension) with a command context. Replaces unipi command runners.
    command-invoke: async func(name: string, args: string) -> result<option<json>, host-error>;
    /// pi ctx.executeTool (tool-call context only). Returns AgentToolCallOutcome JSON.
    execute-tool: async func(name: string, args: json, cancel: option<borrow<cancel-token>>) -> result<json, host-error>;
    /// Report tool progress for the current tool call (pi onUpdate). partial = AgentToolResult JSON.
    tool-progress: func(tool-call-id: string, partial: json);
    /// Detach a running built-in bash call into a background task (permission process.spawn).
    bash-detach: func(tool-call-id: string, reason: string) -> result<string, host-error>;  // task id
    /// Recent foreground bash calls still running: [{toolCallId, command, startedAt, pid}]
    running-bash: func() -> json;
}

// ─────────────────────────── models.wit ─────────────────────────────
interface models {
    use types.{json, host-error, cancel-token};

    /// Non-session completion against any registered model (permission models.complete).
    /// request = oino_protocol::ext::CompletionRequest {model?: "provider/id", context, options}
    /// response = AssistantMessage. Usage is accounted to the session as a `usage` entry kind "ext:<id>".
    complete: async func(request: json, cancel: option<borrow<cancel-token>>) -> result<json, host-error>;
    /// Streaming variant: AssistantMessageEvent JSON values (without `partial`).
    %stream: async func(request: json) -> result<stream<string>, host-error>;
    /// Core's compaction summarizer (C9/C26). entries = Vec<AgentMessage>; returns {summary, usage?}.
    summarize-default: async func(messages: json, custom-instructions: option<string>) -> result<json, host-error>;
    /// Scoped credentials for outbound HTTP (permission credentials:[provider|*]).
    record credential { headers: list<tuple<string, string>>, base-url: option<string>, provider: string }
    credential-for: async func(provider-or-base-url: string) -> result<option<credential>, host-error>;
    /// Classifier / image model calls (pi modelRegistry.classify / image models).
    classify: async func(request: json) -> result<json, host-error>;
    generate-image: async func(request: json) -> result<json, host-error>;
}

// ───────────────────────────── ui.wit ───────────────────────────────
interface ui {
    use types.{json, host-error, cancel-token};
    /// Dialog (blocks until answered, timeout or cancel). request/response = ui-protocol §4 JSON
    /// (method select|confirm|input|editor|multiSelect|form|approval|custom).
    dialog: async func(request: json, cancel: option<borrow<cancel-token>>) -> result<json, host-error>;
    /// Fire-and-forget (notify|setStatus|setWidget|setTitle|set_editor_text|setFrame|setWorkingMessage|
    /// setHeader|setFooter|registerAutocomplete…), ui-protocol §4.2.
    fire: func(request: json) -> result<_, host-error>;
    /// Panels (ui-protocol §5): open returns panel id; update replaces the spec; close.
    panel-open: func(spec: json) -> result<string, host-error>;
    panel-update: func(panel-id: string, spec: json) -> result<_, host-error>;
    panel-close: func(panel-id: string);
    /// Editor model (ui-protocol §7).
    editor-text: func() -> string;
    editor-set: func(text: string);
    editor-insert: func(text: string);
    /// OS / device integration (routed to the active frontend; permissions osNotify, clipboard, openUrl).
    os-notify: func(title: string, body: string, urgency: string, action-url: option<string>) -> result<_, host-error>;
    clipboard-write-text: func(text: string) -> result<_, host-error>;
    clipboard-read-image: async func() -> result<option<json>, host-error>;   // ImageContent
    open-url: func(url: string) -> result<_, host-error>;
    edit-externally: async func(text: string, extension-hint: option<string>) -> result<option<string>, host-error>;
}

// ─────────────────────────── agents.wit ─────────────────────────────
interface agents {
    use types.{json, host-error};
    /// Nested AgentSession (runtime §12). spec = oino_protocol::ext::AgentSpec JSON.
    resource agent {
        spawn: static async func(spec: json) -> result<agent, host-error>;
        id: func() -> string;
        session-file: func() -> option<string>;
        prompt: async func(text: string, delivery: option<string>) -> result<string, host-error>; // disposition
        steer: async func(text: string) -> result<_, host-error>;
        follow-up: async func(text: string) -> result<_, host-error>;
        abort: func();
        set-thinking: func(level: string) -> result<string, host-error>;
        last-assistant-text: func() -> option<string>;
        wait-idle: async func() -> result<_, host-error>;
        usage: func() -> json;                                 // SessionStats subset
        /// ClientEvent JSON stream of the child; also delivered as oino_agent events when subscribed.
        events: func() -> stream<string>;
        close: async func(grace-ms: u32);
    }
    /// Children of the current session: [{id, label, state, sessionFile}]
    list: func() -> json;
}

// ─────────────────────────── process.wit ────────────────────────────
interface process {
    use types.{host-error};
    enum stdio-mode { null, pipe, inherit-log }               // inherit-log = to logs/ext/<id>.log
    variant output-target { null, pipe, file(string) }
    enum signal { term, kill, int, hup }
    record spawn-spec {
        command: string, args: list<string>, cwd: option<string>,
        env: list<tuple<string, string>>, clear-env: bool,
        stdin: stdio-mode, stdout: output-target, stderr: output-target,
        new-process-group: bool,
        /// Keep running after the session/extension shuts down (daemons); permission process.detached.
        detached: bool,
        timeout-ms: option<u64>,
    }
    record exit-status { code: option<s32>, signal: option<s32>, timed-out: bool }
    record exec-result { stdout: string, stderr: string, code: option<s32>, killed: bool }  // pi exec shape
    record exec-options { cwd: option<string>, timeout-ms: option<u64>, env: list<tuple<string, string>> }
    record proc-stats { pid: u32, ppid: u32, state: string, cpu-ms: u64, rss-bytes: u64, read-bytes: u64, write-bytes: u64, start-time-ms: u64 }

    resource child {
        spawn: static func(spec: spawn-spec) -> result<child, host-error>;
        pid: func() -> u32;
        write-stdin: async func(data: list<u8>) -> result<_, host-error>;
        close-stdin: func();
        stdout: func() -> option<stream<u8>>;
        stderr: func() -> option<stream<u8>>;
        wait: async func() -> exit-status;
        try-wait: func() -> option<exit-status>;
        signal: func(sig: signal, group: bool) -> result<_, host-error>;
    }
    /// pi exec(command, args, {timeout, cwd}) → {stdout, stderr, code, killed}.
    exec: async func(command: string, args: list<string>, options: exec-options) -> result<exec-result, host-error>;
    process-tree: func(pid: u32) -> result<list<u32>, host-error>;
    process-stats: func(pid: u32) -> result<proc-stats, host-error>;
    /// The oino process itself (for watchdog child-tree scans).
    self-pid: func() -> u32;
}

// ──────────────────────────── tasks.wit ─────────────────────────────
interface tasks {
    use types.{json, host-error};
    /// Host background-task service (C12). Owned by the host; unipi background-tasks drives it.
    /// spec = oino_protocol::ext::TaskSpec {name, command, isAgent, description?, timeoutSeconds?,
    ///        notifyOnCompletion, triggerOnCompletion, maxOutputBytes?}
    start: func(spec: json) -> result<string, host-error>;     // task id; output file under ~/.oino/tmp/tasks/
    adopt-bash: func(tool-call-id: string, reason: string) -> result<string, host-error>;
    list: func() -> json;                                      // Vec<TaskInfo> (running + last 30 finished)
    get: func(id: string) -> option<json>;
    logs: func(id: string, max-bytes: u32, tail: bool) -> result<string, host-error>;
    kill: func(id: string) -> result<_, host-error>;
    clear-finished: func();
    // Completion: host emits `oino_task` events {id, state, exitCode?}; when triggerOnCompletion the
    // host also delivers <background-task-notification> per unipi-port §3.9 and counts the task as an
    // arbiter wait source while running.
}

// ──────────────────────────── http.wit ──────────────────────────────
interface http {
    use types.{host-error};
    enum method { get, head, post, put, patch, delete, options }
    variant auth { none, provider(string) }                    // inject credential-for(provider) headers
    record request {
        method: method, url: string,
        headers: list<tuple<string, string>>,
        body: option<list<u8>>,
        timeout-ms: option<u32>,
        follow-redirects: bool,
        auth: auth,
        /// TLS/HTTP2 fingerprint impersonation profile, e.g. "chrome_145" (permission network.fingerprint).
        impersonate: option<string>,
        proxy: option<string>,                                  // overrides settings httpProxy
    }
    record response-head { status: u16, headers: list<tuple<string, string>>, final-url: string, impersonated: bool }
    /// Streaming response; body ends when the stream closes.
    fetch: async func(req: request) -> result<tuple<response-head, stream<u8>>, host-error>;
    /// Loopback-only HTTP server (permission network.listenLoopback). port 0 = ephemeral.
    record incoming { id: u64, method: string, path: string, headers: list<tuple<string, string>>, body: list<u8> }
    resource listener {
        bind: static func(port: u16) -> result<listener, host-error>;
        port: func() -> u16;
        accept: async func() -> option<incoming>;
        respond: func(id: u64, status: u16, headers: list<tuple<string, string>>, body: list<u8>) -> result<_, host-error>;
    }
}

// ───────────────────────────── fs.wit ───────────────────────────────
interface fs {
    use types.{host-error};
    /// Named preopens available to this extension (§5.8): "ext-global", "ext-workspace",
    /// "ext-session", "project", "tmp", "cache", "logs", plus manifest fs.paths aliases.
    record preopen { name: string, guest-path: string, host-path: string, writable: bool }
    preopens: func() -> list<preopen>;
    atomic-write: func(path: string, data: list<u8>, mode: option<u32>) -> result<_, host-error>;
    resource lock {
        acquire: static async func(path: string, exclusive: bool) -> result<lock, host-error>;
        release: func();
    }
    record fs-event { kind: string, path: string }            // kind: create|modify|remove|rename
    resource watcher {
        watch: static func(path: string, recursive: bool) -> result<watcher, host-error>;
        next: async func() -> option<list<fs-event>>;
    }
}

// ──────────────────────────── timers.wit ────────────────────────────
interface timers {
    /// Fires `oino_timer{timerId}` events into on-event. Same id replaces.
    set: func(id: string, delay-ms: u64, repeat: bool);
    cancel: func(id: string);
    sleep: async func(ms: u64);
    now-ms: func() -> s64;                                     // wall clock, epoch ms
}

// ───────────────────────────── kv.wit ───────────────────────────────
interface kv {
    use types.{host-error};
    enum scope { global, workspace, session }
    get: func(scope: scope, key: string) -> option<list<u8>>;
    set: func(scope: scope, key: string, value: list<u8>) -> result<_, host-error>;
    delete: func(scope: scope, key: string);
    keys: func(scope: scope, prefix: string) -> list<string>;
}

// ─────────────────────────── secrets.wit ────────────────────────────
interface secrets {
    use types.{host-error};
    get: func(name: string) -> result<option<string>, host-error>;
    set: func(name: string, value: string) -> result<_, host-error>;
    delete: func(name: string) -> result<_, host-error>;
}

// ───────────────────────────── bus.wit ──────────────────────────────
interface bus {
    use types.{json};
    publish: func(topic: string, data: json, sticky: bool);
    /// pattern: exact topic or "prefix:*". Deliveries arrive as oino_bus events.
    subscribe: func(pattern: string);
    unsubscribe: func(pattern: string);
    get: func(topic: string) -> option<json>;                 // last sticky value
}

// ─────────────────────────── services.wit ───────────────────────────
interface services {
    use types.{json, extension-id, host-error};
    /// Call a service registered by any extension (or a specific one). Timeout default 5000 ms.
    call: async func(target: option<extension-id>, name: string, payload: json, timeout-ms: option<u32>) -> result<json, host-error>;
    list: func() -> list<tuple<extension-id, string>>;
}

// ──────────────────────────── guest.wit ─────────────────────────────
interface guest {
    use types.{json, call-context, cancel-token};
    use registry.{registration, event-kind};

    record init-input {
        /// Manifest JSON (as installed) and the extension's resolved settings for its namespaces.
        manifest: json,
        settings: json,
        flags: json,
    }
    record tool-invocation { tool-call-id: string, name: string, args: json, parent-tool-call-id: option<string> }
    record completion-item { value: string, label: option<string>, description: option<string> }

    init: async func(input: init-input, ctx: call-context) -> result<registration, string>;
    /// Event dispatch. payload/result = JSON of oino_protocol::ext::events::<Event>/<Result>.
    /// Return none for "no result" (pi `undefined`).
    on-event: async func(kind: event-kind, payload: json, ctx: call-context) -> result<option<json>, string>;
    /// Returns AgentToolResult JSON. Progress via control.tool-progress.
    run-tool: async func(invocation: tool-invocation, cancel: borrow<cancel-token>, ctx: call-context) -> result<json, string>;
    prepare-arguments: func(tool: string, raw: json) -> json;
    prepare-loadout: func(tool: string, loadout: json) -> option<json>;
    reaches-tools: func(tool: string, input: json) -> json;   // "all" | ["name", …]
    run-command: async func(name: string, args: string, cancel: borrow<cancel-token>, ctx: call-context) -> result<option<json>, string>;
    complete-command: async func(name: string, prefix: string, ctx: call-context) -> list<completion-item>;
    run-shortcut: async func(id: string, ctx: call-context) -> result<_, string>;
    handle-service: async func(name: string, payload: json, caller: string, ctx: call-context) -> result<json, string>;
    /// Arbiter (runtime §7.6). Nudge JSON = {id, priority, customType, content, display, details?}.
    propose-nudge: async func(ctx: call-context) -> option<json>;
    nudge-delivered: func(id: string, ctx: call-context);
    wait-reason: async func(ctx: call-context) -> option<string>;
    /// Render specs (ui-protocol §6.4). request = RenderRequest JSON; result = OUS node JSON.
    render: func(request: json, ctx: call-context) -> option<json>;
    /// Virtual model routing (runtime §13). request/response = pi route request/result JSON.
    route-model: async func(request: json, ctx: call-context) -> result<json, string>;
    /// Autocomplete contributor (ui-protocol §7.3). request = {text, cursor}; result = Vec<CompletionGroup>.
    complete-input: async func(request: json, ctx: call-context) -> option<json>;
    shutdown: async func(reason: string);
}

// ─────────────────────────── provider.wit ───────────────────────────
interface provider {
    use types.{json};
    /// request = oino_protocol::ext::ProviderStreamRequest {requestId, model, context, options, apiKey?, headers, wantStreamEvents}
    /// Each stream item = ProviderEvent JSON (AssistantMessageEvent without `partial`).
    /// The stream must end with `done` or `error`; dropping the reader cancels.
    %stream: async func(request: json) -> result<stream<string>, string>;
    /// Vec<Model> for a provider id this component registered.
    list-models: async func(provider-id: string, credential: option<json>) -> result<json, string>;
    /// OAuth (pi provider oauth): uses ui + http imports; returns stored credential JSON.
    oauth-login: async func(provider-id: string) -> result<json, string>;
    oauth-refresh: async func(provider-id: string, credential: json) -> result<json, string>;
}

// ───────────────────────── provider-host.wit ────────────────────────
/// Imported by provider components so core can run the provider-level extension events
/// (pi onPayload / onResponse / onProviderStreamEvent) on the real wire payload.
interface provider-host {
    use types.{json};
    /// Call after building the HTTP request, before sending. Core dispatches
    /// before_provider_headers (headers) then before_provider_request (payload); returns the
    /// possibly replaced payload and headers. request-id = the `requestId` from the stream request.
    before-request: async func(request-id: string, payload: json, headers: list<tuple<string, string>>) -> tuple<json, list<tuple<string, string>>>;
    /// Call when response headers arrive (after_provider_response).
    after-response: func(request-id: string, status: u16, headers: list<tuple<string, string>>);
    /// Call for each raw provider stream event when the request asked for it (provider_stream_event).
    stream-event: func(request-id: string, data: json);
}

// ──────────────────────────── worlds.wit ────────────────────────────
world extension {
    import wasi:cli/environment@0.2.3;
    import wasi:cli/stdout@0.2.3;
    import wasi:cli/stderr@0.2.3;
    import wasi:clocks/monotonic-clock@0.2.3;
    import wasi:clocks/wall-clock@0.2.3;
    import wasi:random/random@0.2.3;
    import wasi:filesystem/types@0.2.3;
    import wasi:filesystem/preopens@0.2.3;
    import log; import host-info; import registry; import session; import control;
    import models; import ui; import agents; import process; import tasks; import http;
    import fs; import timers; import kv; import secrets; import bus; import services;
    export guest;
}

world provider-extension {
    include extension;
    import provider-host;
    export provider;
}
```

Notes binding on the host: `wasi:cli/environment` exposes **only** env vars granted by
`permissions.env` (plus `OINO_EXTENSION_ID`); stdout/stderr go to `logs/ext/<id>.log`. WASI
version pinned to the one in `wit/deps` (`0.2.3` shown; integration may bump the patch).

## 4. Data contracts carried as JSON (`oino_protocol::ext`)

### 4.1 Events and results

One struct per event and per result, exact pi field names (runtime §7.2 table). Payloads
include the `type` field (pi `type` strings), so `on-event` handlers can decode either by
`kind` or by the payload. `oino_*` events:

| Event | Payload |
|---|---|
| `oino_timer` | `{type, timerId}` |
| `oino_bus` | `{type, topic, data, sticky, publisher}` |
| `oino_ui_action` | `{type, panelId, actionId, value?}` (ui-protocol §5.3) |
| `oino_editor` | `{type, intent: "changed"\|"cleared"\|"emptyArrowDown"\|"submit"\|"tokenBackspace", text, tokenId?}` (ui-protocol §7) |
| `oino_settings_changed` | `{type, namespace, keys}` |
| `oino_process` | `{type, pid, event: "exit", status}` (for spawned children with `subscribe`) |
| `oino_agent` | `{type, agentId, event: ClientEvent}` |
| `oino_task` | `{type, id, state: "running"\|"done"\|"failed"\|"killed", exitCode?}` |

### 4.2 Tool results

`AgentToolResult{content, details?, structuredContent?, usage?, isError, terminate}`. Image
content allowed. A tool returning `Err(string)` produces pi's error result with that text.

### 4.3 Completion and agent specs

`CompletionRequest{model?: "provider/id" (default: session model), context: {messages},
options: StreamOptions, cacheWrites?: bool}`; `AgentSpec` = runtime §12 in camelCase JSON.

### 4.4 Provider registration

```json
{ "id": "openrouter", "name": "OpenRouter", "api": "openai-completions",
  "baseUrl": "https://openrouter.ai/api/v1", "authEnv": ["OPENROUTER_API_KEY"],
  "authHeader": "bearer", "oauth": false, "headers": {}, "compat": {},
  "models": "dynamic" }
```

`api` values a component may *implement* are declared in its manifest `provides.apis`
(e.g. `["openai-completions"]`). `models: "dynamic"` ⇒ host calls `list-models`; an array ⇒
static `Model` list. Config-only overrides from non-provider extensions may set `baseUrl`,
`headers`, `models` for an existing provider id (pi `registerProvider(name, config)`).

## 5. Host implementation (`oino-wasm-host`)

### 5.1 Engine

wasmtime `=48.0.2` with `component-model`, `component-model-async`, `async`, `cranelift`,
`cache`, `pooling-allocator`; epoch interruption (tick 10 ms); compiled-component cache in
`~/.oino/cache/wasmtime/` keyed by component sha256 + wasmtime version; bundled components are
pre-compiled on first run in the background. Limits per instance: memory 512 MiB default
(manifest `limits.memoryMiB`, max 2048), table elements 100 000, instances 10.
Fuel is not used; deadlines (runtime §7.2) are enforced with epoch deadlines + tokio timeouts.
A trapped instance is restarted lazily on the next call (max 3 restarts per 10 min, then the
extension is disabled for the session with an `extension_error`).

### 5.2 Mapping to `oino-extension-api`

`WasmRuntime: ExtensionRuntime`; `WasmInstance: ExtensionInstance`. Imports that touch
sessions, UI, models, agents, settings, bus, services, secrets, registry, tasks delegate to
`HostServices` (§6) with the caller's `ExtensionId`; `process`, `http`, `fs`, `timers`
(timer wheel per instance, firing through `HostServices::deliver_event`), `kv` (one JSON file
per scope, `kv.json` inside the scope's preopen directory, written atomically, max 16 MiB),
`log`, `host-info` and `provider-host` (forwarded to `HostServices::provider_hook`) are
implemented in the wasm host itself.

### 5.3 Permission enforcement

Every import checks the manifest grant before doing work and returns
`host-error::denied("<permission>")`. Grants are fixed at load; the package manager shows them
at install time (runtime §15). Bundled extensions' grants are trusted (shipped with Oino).

### 5.4 Process service

`tokio::process`, own process group when `new-process-group`, tree kill (Unix `killpg`,
Windows Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), stats via `/proc` (Linux),
`proc_pidinfo` via the `libproc` crate (macOS), `sysinfo` crate fallback (Windows). Detached
children are recorded in `~/.oino/state/<id>/global/daemons.json` (pid + start time) so they
can be found on the next run.

### 5.5 Tasks service

Implements `tasks` (C12) using the process service; output to
`~/.oino/tmp/tasks/<session8>-<taskid>/output.log`; max output 20 MiB default
(`maxOutputBytes`, min 1 KiB) — over the cap the task is killed and fails; keeps the last 30
finished tasks per session in memory and `~/.oino/state/oino.tasks/session/<sid>/tasks.json`.
On completion with `notifyOnCompletion`, core delivers the notification message (unipi
background-tasks defines the text; the host calls the owning extension's service
`tasks.notification` to render it, falling back to `Background task {name} finished with exit
code {code}`).

### 5.6 HTTP

`reqwest` (rustls) for normal requests; `wreq` with the requested emulation profile when
`impersonate` is set and cargo feature `fingerprint` is on (else plain request with
`impersonated: false`). Allowlist from `permissions.network.hosts` (exact host or `*.domain`;
`"*"` allowed only for bundled extensions). Proxy from settings `httpProxy` / env. Default
timeout 300 s idle (`httpIdleTimeoutMs`). Providers get their base URL hosts automatically.

### 5.7 Filesystem preopens

| Name | Host path | Access |
|---|---|---|
| `ext-global` | `~/.oino/state/<id>/global/` | rw (always) |
| `ext-workspace` | `~/.oino/state/<id>/workspace/<wsid>/` | rw (always) |
| `ext-session` | `~/.oino/state/<id>/session/<sid>/` | rw (always; GC'd when the session file no longer exists, at startup) |
| `tmp` | `~/.oino/tmp/ext/<id>/` | rw (always) |
| `cache` | `~/.oino/cache/ext/<id>/` | rw (always) |
| `logs` | `~/.oino/logs/ext/<id>/` | rw (always) |
| `project` | cwd | `permissions.fs.project`: `none` (default) \| `read` \| `write` |
| `agent` | `~/.oino/` | `permissions.fs.agentDir`: `none` \| `read` (sessions, trust, models — never `auth.json`/`secrets.json`, which are masked) |
| alias names | `permissions.fs.paths[].path` (`~` expanded) | as declared (`read`\|`write`) |

Guest paths are `/<name>` (e.g. `/ext-global/hints.json`). The SDK `paths` module resolves
them.

### 5.8 State and workspace identity (D10)

`wsid` = first 16 hex chars of `sha256(canonical project root)`; project root = `git rev-parse
--show-toplevel` equivalent (walk up for `.git`), else cwd. Moving a project changes `wsid`;
`/unipi:move` (utility) calls each extension's `move` service to migrate.

### 5.9 Secrets

`~/.oino/secrets.json` (0600, file lock), keys namespaced `<ext-id>/<name>`; extensions only
see their own namespace. Settings fields of type `secret` read/write through this store.

## 6. `oino-extension-api` crate (Rust contracts)

```rust
pub type BoxFut<'a, T> = futures::future::BoxFuture<'a, T>;
pub struct ExtensionId(String);
pub struct RawJson(pub Box<serde_json::value::RawValue>);

pub trait ExtensionRuntime: Send + Sync {
    fn load(&self, package: LoadedPackage, host: Arc<dyn HostServices>) -> BoxFut<'_, Result<Arc<dyn ExtensionInstance>, ExtensionError>>;
}
pub struct LoadedPackage { pub manifest: Manifest, pub component: ComponentSource /* Bytes(Arc<[u8]>) | Path */, pub root: Option<PathBuf> }

pub trait ExtensionInstance: Send + Sync {
    fn id(&self) -> &ExtensionId;
    fn manifest(&self) -> &Manifest;
    fn init(&self, input: InitInput, cx: CallContext) -> BoxFut<'_, Result<Registration, ExtensionError>>;
    fn on_event(&self, kind: EventKind, payload: RawJson, cx: CallContext) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>>;
    fn run_tool(&self, inv: ToolInvocation, cancel: CancellationToken, cx: CallContext) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    fn prepare_arguments(&self, tool: &str, raw: RawJson) -> Result<RawJson, ExtensionError>;
    fn prepare_loadout(&self, tool: &str, loadout: RawJson) -> Result<Option<RawJson>, ExtensionError>;
    fn reaches_tools(&self, tool: &str, input: RawJson) -> Result<RawJson, ExtensionError>;
    fn run_command(&self, name: &str, args: &str, cancel: CancellationToken, cx: CallContext) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>>;
    fn complete_command(&self, name: &str, prefix: &str, cx: CallContext) -> BoxFut<'_, Vec<CompletionItem>>;
    fn run_shortcut(&self, id: &str, cx: CallContext) -> BoxFut<'_, Result<(), ExtensionError>>;
    fn handle_service(&self, name: &str, payload: RawJson, caller: &ExtensionId, cx: CallContext) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    fn propose_nudge(&self, cx: CallContext) -> BoxFut<'_, Option<RawJson>>;
    fn nudge_delivered(&self, id: &str, cx: CallContext);
    fn wait_reason(&self, cx: CallContext) -> BoxFut<'_, Option<String>>;
    fn render(&self, request: RawJson, cx: CallContext) -> Option<RawJson>;
    fn route_model(&self, request: RawJson, cx: CallContext) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    fn complete_input(&self, request: RawJson, cx: CallContext) -> BoxFut<'_, Option<RawJson>>;
    fn provider(&self) -> Option<&dyn ProviderInstance>;
    fn shutdown(&self, reason: &str) -> BoxFut<'_, ()>;
}
pub trait ProviderInstance: Send + Sync {
    fn stream(&self, request: RawJson) -> BoxFut<'_, Result<BoxStream<'static, Result<ProviderEvent, ExtensionError>>, ExtensionError>>;
    fn list_models(&self, provider_id: &str, credential: Option<RawJson>) -> BoxFut<'_, Result<Vec<Model>, ExtensionError>>;
    fn oauth_login(&self, provider_id: &str) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    fn oauth_refresh(&self, provider_id: &str, credential: RawJson) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
}

/// Implemented by oino-core. One method per WIT import that needs the runtime; argument and
/// return types are the Rust equivalents of the WIT signatures in §3 (records → structs,
/// json → RawJson, host-error → HostError). Grouped by WIT interface.
pub trait HostServices: Send + Sync {
    // registry
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]);
    fn unsubscribe(&self, ext: &ExtensionId, events: &[EventKind]);
    fn register_tool(&self, ext: &ExtensionId, spec: ToolSpec) -> Result<(), HostError>;
    fn unregister_tool(&self, ext: &ExtensionId, name: &str);
    fn register_command(&self, ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError>;
    fn unregister_command(&self, ext: &ExtensionId, name: &str);
    fn register_shortcut(&self, ext: &ExtensionId, spec: ShortcutSpec) -> Result<(), HostError>;
    fn unregister_shortcut(&self, ext: &ExtensionId, id: &str);
    fn register_flag(&self, ext: &ExtensionId, spec: FlagSpec) -> Result<(), HostError>;
    fn get_flag(&self, ext: &ExtensionId, name: &str) -> Option<RawJson>;
    fn register_renderer(&self, ext: &ExtensionId, spec: RendererSpec);
    fn register_service(&self, ext: &ExtensionId, name: &str) -> Result<(), HostError>;
    fn set_arbiter_roles(&self, ext: &ExtensionId, roles: ArbiterRoles);
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError>;
    fn register_provider(&self, ext: &ExtensionId, cfg: RawJson) -> Result<(), HostError>;
    fn unregister_provider(&self, ext: &ExtensionId, id: &str);
    fn register_virtual_model(&self, ext: &ExtensionId, def: RawJson) -> Result<(), HostError>;
    fn unregister_virtual_model(&self, ext: &ExtensionId, provider: &str, id: &str);
    // session
    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError>;   // header|entries|branch|entry|tree|projection|context-entries|label|name|leaf|id|file
    fn session_write(&self, ext: &ExtensionId, cx: &CallContext, w: SessionWrite) -> BoxFut<'_, Result<Option<String>, HostError>>; // set-name|set-label|append-entry|send-message|send-user-message
    fn sessions_index(&self, ext: &ExtensionId, q: SessionsIndexQuery) -> BoxFut<'_, Result<RawJson, HostError>>;
    // control
    fn control(&self, ext: &ExtensionId, cx: &CallContext, op: ControlOp) -> BoxFut<'_, Result<ControlReply, HostError>>;
    fn tool_progress(&self, cx: &CallContext, tool_call_id: &str, partial: RawJson);
    // models
    fn models(&self, ext: &ExtensionId, cx: &CallContext, op: ModelsOp) -> BoxFut<'_, Result<ModelsReply, HostError>>;
    // ui
    fn ui_dialog(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson, cancel: Option<CancellationToken>) -> BoxFut<'_, Result<RawJson, HostError>>;
    fn ui_fire(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson) -> Result<(), HostError>;
    fn ui_panel(&self, ext: &ExtensionId, cx: &CallContext, op: PanelOp) -> Result<Option<String>, HostError>;
    fn ui_editor(&self, cx: &CallContext, op: EditorOp) -> Result<Option<String>, HostError>;
    fn ui_device(&self, ext: &ExtensionId, cx: &CallContext, op: DeviceOp) -> BoxFut<'_, Result<Option<RawJson>, HostError>>; // os-notify|clipboard|open-url|edit-externally
    // agents
    fn agent_spawn(&self, ext: &ExtensionId, cx: &CallContext, spec: RawJson) -> BoxFut<'_, Result<Arc<dyn AgentHandle>, HostError>>;
    fn agents_list(&self, cx: &CallContext) -> RawJson;
    // tasks
    fn tasks(&self, ext: &ExtensionId, cx: &CallContext, op: TaskOp) -> Result<RawJson, HostError>;
    // bus / services / secrets / settings
    fn bus_publish(&self, ext: &ExtensionId, cx: &CallContext, topic: &str, data: RawJson, sticky: bool);
    fn bus_subscribe(&self, ext: &ExtensionId, cx: &CallContext, pattern: &str, on: bool);
    fn bus_get(&self, cx: &CallContext, topic: &str) -> Option<RawJson>;
    fn service_call(&self, ext: &ExtensionId, cx: &CallContext, target: Option<&ExtensionId>, name: &str, payload: RawJson, timeout: Duration) -> BoxFut<'_, Result<RawJson, HostError>>;
    fn services_list(&self) -> Vec<(ExtensionId, String)>;
    fn secret(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError>;
    // provider-host (dispatches before_provider_headers/request, after_provider_response, provider_stream_event)
    fn provider_hook(&self, ext: &ExtensionId, hook: ProviderHook) -> BoxFut<'_, ProviderHookReply>;
    // host info pieces that need the runtime
    fn host_context(&self, cx: &CallContext) -> HostContextInfo;   // frontends, focused, workspace
    // events produced by the wasm host itself (timers, process exits)
    fn deliver_event(&self, ext: &ExtensionId, session_id: &str, kind: EventKind, payload: RawJson);
}
pub trait AgentHandle: Send + Sync { /* mirrors WIT agents.agent methods */ }
```

The enums `ControlOp`, `ModelsOp`, `SessionQuery`, `SessionWrite`, `PanelOp`, `EditorOp`,
`DeviceOp`, `TaskOp`, `SecretOp` have exactly one variant per WIT function of the
corresponding interface (same names in UpperCamelCase, same parameters). `ExtensionError`
carries `{extension, call, message, trap: bool}`. Module `oino_extension_api::testing`
(feature `testing`) provides `FakeExtensionRuntime` (scriptable instances implemented in Rust
closures) and `RecordingHostServices`; core-runtime uses these in unit tests.

Manifest types (§7) live in `oino_extension_api::manifest`.

## 7. Packages and manifests

### 7.1 Package layout

```text
<package>/
  extension.json        # manifest (required)
  component.wasm        # component (required)
  skills/ prompts/ themes/ docs/   # optional resources (declared in manifest)
```

### 7.2 `extension.json`

```json
{
  "apiVersion": "oino.extension/1",
  "id": "unipi.background-tasks",
  "name": "Background tasks",
  "version": "3.0.0",
  "kind": "extension",
  "description": "Run shell commands in the background and wake the agent when they finish.",
  "license": "MIT",
  "authors": ["…"],
  "component": "component.wasm",
  "checksums": { "component.wasm": "sha256:…" },
  "minOinoVersion": "0.1.0",
  "provides": { "apis": [], "providers": [] },
  "resources": { "skills": ["skills"], "prompts": [], "themes": [], "docs": "docs" },
  "limits": { "memoryMiB": 512, "hookTimeoutMs": 5000, "toolCallHookTimeoutMs": 30000 },
  "permissions": { "…": "see 7.3" }
}
```

Validation (`oino ext validate <dir>`, runtime package manager, xtask): `apiVersion` must be
`oino.extension/1`; `id` regex (types.wit); `version` semver; `kind` ∈ `extension|provider`;
provider kind requires `provides.apis` non-empty and the component to export `provider`
(checked by inspecting the component's world); declared resources exist; checksums match.

### 7.3 Permissions

| Permission (JSON path) | Type / values | Gates |
|---|---|---|
| `sessionControl` | bool | control setters/actions: set-model, set-thinking-level, abort, compact, shutdown, set-active-tools, session writes other than append-entry/send-message |
| `sessions.readAll` | bool | `session.sessions-list`, `session-read` |
| `tools.override` | list of built-in tool names | re-registering render metadata of built-ins |
| `models.complete` | bool | `models.complete`, `stream`, `summarize-default`, `classify`, `generate-image` |
| `credentials` | list of provider ids or `"*"` | `models.credential-for`, `http` `auth: provider(..)` |
| `agents.spawn` | bool; `agents.maxDepth` u8 | `agents` interface |
| `process.spawn` | bool; `process.allow` list of program names/globs (`"*"` bundled only) | `process.*`, `tasks.*`, `control.bash-detach` |
| `process.detached` | bool | `spawn-spec.detached` |
| `network.hosts` | list of hosts / `*.domain` | `http.fetch` |
| `network.fingerprint` | bool | `request.impersonate` |
| `network.listenLoopback` | bool | `http.listener` |
| `fs.project` | `none\|read\|write` | `project` preopen |
| `fs.agentDir` | `none\|read` | `agent` preopen |
| `fs.paths` | `[{path, access, alias}]` | extra preopens |
| `env` | list of names | `host-info.env-get`, WASI environment |
| `secrets` | bool | `secrets.*` |
| `ui` | bool (default true) | `ui.dialog`, `ui.fire`, panels, editor |
| `osNotify` / `openUrl` / `clipboard.read` / `clipboard.write` / `editExternally` | bool | `ui` device functions |
| `services.call` | list of `<ext-id>/<name>` or `"*"` | `services.call` (registering is always allowed) |

Default for every permission: denied (except `ui`).

## 8. Bundling (D14)

1. `cargo xtask wasm` builds every crate under `extensions/{providers,unipi,fixtures}` for
   `wasm32-wasip2` (`--release`, profile `opt-level="s"`, `lto=true`, `strip=true`,
   `codegen-units=1`), runs `wasm-tools component new` when needed, and writes
   `extensions/dist/<id>/{extension.json, component.wasm, <resources>}` plus
   `extensions/dist/checksums.json` (sha256 per file + toolchain + wasm-tools versions).
   Only `checksums.json` is committed; CI rebuilds and diffs it.
2. `crates/oino-bundle/build.rs` reads `extensions/dist/` (path override env
   `OINO_BUNDLE_DIST`) and emits a static table:
   `pub static BUNDLED: &[BundledExtension]` with `{id, version, manifest: &str, component:
   &[u8] (zstd-compressed, decompressed lazily), resources: &[(&str, &[u8])], hash8}` and the
   docs pseudo-package `oino.docs` (repo `README.md`, `docs/`, `examples/`). Cargo feature
   `providers` and `unipi` (both default) select sets; `oino-bundle` without dist (fresh clone)
   builds an empty table and emits a cargo warning.
3. At runtime `BundledSource` (trait in oino-core) extracts resources to
   `~/.oino/bundled/<id>@<version>-<hash8>/` once (atomic dir rename) and hands components to
   the runtime as bytes.

## 9. Guest SDK (`extensions/sdk/oino-extension`)

### 9.1 Shape

```rust
use oino_extension::prelude::*;

#[derive(Default)]
struct Notify { state: RefCell<State> }

impl Extension for Notify {
    async fn init(&self, cx: &mut Init) -> Result<()> {
        cx.on(Events::AgentSettled, Self::on_settled);           // typed: fn(&Self, AgentSettledEvent, Ctx) -> impl Future<Output = Result<()>>
        cx.on(Events::ToolCall, Self::gate);                      // returns ToolCallResult
        cx.tool(Tool::new("notify_user")
            .label("Notify user")
            .description("Send a notification to the user's configured platforms.")
            .args::<NotifyArgs>()                                 // schemars JsonSchema → parameters
            .kind(ToolKind::Other)
            .run(Self::notify_user));
        cx.command(Command::new("unipi:notify-event").description("Toggle an event").run(Self::cmd_event).complete(Self::complete_event));
        cx.settings(SettingsNamespace::new("notify").schema(schema()).defaults(defaults()));
        cx.timer("renotify", Duration::from_secs(120), Repeat::Yes);
        Ok(())
    }
}

#[derive(Deserialize, JsonSchema)]
struct NotifyArgs { message: String, title: Option<String>, priority: Option<Priority> }

impl Notify {
    async fn notify_user(&self, args: NotifyArgs, cx: ToolCtx) -> Result<ToolResult> {
        cx.progress(ToolResult::text("sending…"));
        let resp = http::get("https://ntfy.sh/…").header("Title", &title).send().await?;
        Ok(ToolResult::text(format!("Sent ({})", resp.status())))
    }
    async fn on_settled(&self, ev: AgentSettledEvent, cx: Ctx) -> Result<()> { /* … */ Ok(()) }
    async fn gate(&self, ev: ToolCallEvent, cx: Ctx) -> Result<Option<ToolCallResult>> { Ok(None) }
}

oino_extension::export_extension!(Notify);
```

### 9.2 Modules

| Module | Provides |
|---|---|
| `prelude` | `Extension`, `Init`, `Ctx`, `ToolCtx`, `CommandCtx`, `Result`, `Error`, common types |
| `events` | `Events::*` typed keys (one per event kind) binding payload/result types from `oino_protocol::ext::events` |
| `tool` | `Tool` builder, `ToolResult` helpers (`text`, `error`, `with_details`, `image`), progress |
| `command` | `Command`, `Shortcut`, `Flag` builders |
| `session` | typed wrappers over `session` (entries as `oino_session` types), `send_message`, `append_entry` |
| `control`, `models`, `agents`, `process`, `tasks`, `http`, `fs`, `timers`, `kv`, `secrets`, `bus`, `services` | thin async wrappers over the imports with typed errors |
| `ui` | typed `Dialog::select/confirm/input/editor/multi_select/form/approval/custom`, `notify`, `status`, `widget`, `frame`, `panel` builders producing OUS nodes (`ui::spec` mirrors `oino_protocol::ui::ous`) |
| `settings` | schema builder, typed read/write |
| `paths` | preopen-relative path helpers (`ext_global()`, `ext_workspace()`, …), atomic JSON read/write |
| `testing` | `MockHost` (feature `testing`, native target): runs an `Extension` impl against an in-memory host for unit tests without wasm |

Rules: SDK state lives in the `Extension` value (one per instance, `export_extension!` stores it
in a `OnceCell`); single-threaded executor from wit-bindgen; `RefCell`/`async` locks
(`oino_extension::sync::Mutex`) for shared state across concurrent calls; per-session state
helper `SessionMap<T>` keyed by `Ctx::session_id()`. Errors: `oino_extension::Error`
(`thiserror`) → `Err(string)` at the boundary with the display text.

### 9.3 Testing extensions

Each component crate has native unit tests via `testing::MockHost` (the SDK compiles for the
host target with feature `testing`, replacing imports with the mock). Integration tests run the
built component in `oino-wasm-host`'s test harness (`oino_wasm_host::testing::Harness`) against
`RecordingHostServices`.

## 10. Coverage tables

### 10.1 pi `ExtensionAPI` / `ctx` → WIT

| pi | WIT |
|---|---|
| `on` | `registry.subscribe`, `registration.events`, `guest.on-event` |
| `registerTool` (+ exposure, namespace, reachesTools, prepareLoadout, prepareArguments, outputSchema, annotations, defaultActive, executionMode, renderCall/renderResult) | `registry.register-tool` + `guest.run-tool/prepare-*/reaches-tools/render` |
| `registerCommand` + `getArgumentCompletions` | `register-command` + `complete-command`, `run-command` |
| `registerShortcut` | `register-shortcut` + `run-shortcut` |
| `registerFlag`, `getFlag` | `register-flag`, `get-flag` |
| `registerMessageRenderer`/`EntryRenderer`/`ToolRenderer` | `register-renderer` + `guest.render` |
| `registerMarkdownTransformer` | not supported (TUI-only in pi; frontends own markdown) |
| `sendMessage`, `sendUserMessage`, `appendEntry` | `session.send-message`, `send-user-message`, `append-entry` |
| `setSessionName`/`getSessionName`, `setLabel` | `session.set-session-name`, `session-name`, `set-label` |
| `exec` | `process.exec` |
| `getActiveTools`, `setActiveTools`, `getAllTools` | `control.active-tools`, `set-active-tools`, `all-tools` |
| `getSettings`, `getCommands` | `control.get-settings`, `commands-list` |
| `setModel`, `get/setThinkingLevel` | `control.set-model`, `thinking-level`, `set-thinking-level` |
| `registerProvider`/`unregisterProvider` | `registry.register-provider`/`unregister-provider` + `provider` exports |
| `registerVirtualModel`/`unregister…` | `registry.register-virtual-model` + `guest.route-model` |
| `registerMcpServer` & co | not in v1 (D22: MCP is an extension) |
| `events` bus | `bus.*` |
| `ctx.ui.*` | `ui.*` (ui-protocol §9 maps every method) |
| `ctx.mode`, `hasUI`, `cwd`, `signal` | `call-context`, `cancel-token` |
| `ctx.sessionManager` | `session` reads |
| `ctx.modelRegistry` (find, streamSimple, classify, auth) | `control.models-list`, `models.*`, `credential-for` |
| `ctx.model`, `scopedModels`, `thinkingLevel` | `control.model`, `scoped-models`, `thinking-level` |
| `ctx.isIdle`, `isProjectTrusted`, `abort`, `hasPendingMessages`, `shutdown`, `getContextUsage`, `compact`, `getSystemPrompt` | `control.*` |
| `ctx.tools`, `executeTool` | `control.all-tools`, `execute-tool` |
| `ExtensionCommandContext.*` | `control` command-context-only functions |
| `ReplacedSessionContext` | after a replacing op, new calls carry the new `session-id`; old context calls fail `stale-context` |

### 10.2 unipi capabilities → WIT

| Cap | WIT |
|---|---|
| C1 | `registry` events + `guest.on-event`, deadlines (runtime §7.2) |
| C2 | `register-tool`, async `run-tool`, `cancel-token`, `tool-progress` |
| C3 | `control.active-tools/set-active-tools/all-tools`, `before_provider_request` |
| C4 | `register-command`, `complete-command`, `control.command-invoke`, `commands-list`, host command echo |
| C5 | `register-shortcut`, `ui.editor-*`, `oino_editor` intents (ui-protocol §7) |
| C6 | `session.send-message/send-user-message/append-entry` + `provenance` |
| C7 | `session` reads + `sessions-list`/`session-read` |
| C8 | `control` |
| C9 | `models.complete/stream/summarize-default/credential-for` |
| C10 | `agents` |
| C11 | `process`, `control.bash-detach/running-bash` |
| C12 | `tasks` |
| C13 | `timers` |
| C14 | WASI fs + named preopens + `fs` helpers |
| C15 | `http` (fetch, fingerprint, listener) |
| C16 | `bus`, `services`, arbiter exports |
| C17 | `ui` + OUS (ui-protocol) |
| C18 | `registry.register-settings`, settings hub (runtime §10.3) |
| C19 | `ui.os-notify`, `host-info.frontend-focused` |
| C20 | `ui.clipboard-*` |
| C21 | `ui.open-url`, `ui.edit-externally` |
| C22 | `secrets`, `credential-for`, `host-info.env-get` |
| C23 | `host-info` |
| C24 | `control` command-context functions |
| C25 | typed `before_agent_start` / `resources_discover` / `context` results |
| C26 | `session_before_compact`, boundary compaction drafts, `summarize-default` |
| C27 | `AgentSpec.tools`, `control.execute-tool` |
| C28 | `propose-nudge`, `wait-reason`, `nudge-delivered` |
| C29 | automatic child usage entries; `control.session-stats` |

## 11. Fixtures (`extensions/fixtures`)

| Fixture | Purpose |
|---|---|
| `echo` | registers a tool, a command with completions, a shortcut, a flag, subscribes to every event and records payloads into `append-entry`; used by host and core integration tests |
| `provider-faux` | provider component reading a **faux script** (quality §4.1) from its request `metadata.fauxScript` or a bundled default; deterministic streaming incl. tool calls, errors, overflow |
| `ui-probe` | issues every dialog/fire/panel request; asserts responses |
| `async-probe` | long-running tool waiting on `ui.dialog` while events are dispatched concurrently; timers; cancellation; services between two instances |
| `agent-probe` | nested agents (`delegate` tool, `spawn`/`close` commands over `AgentSpec` JSON), arbiter nudge provider + wait source, `control.run-state` / `shutdown` |

## 12. Provider components (`providers-v1`)

| Component id | Dir | `provides.apis` | Providers registered (from bundled `catalog.json`) | Auth |
|---|---|---|---|---|
| `oino.openai-compat` | `extensions/providers/openai-compat` | `openai-completions` | every OpenAI-compatible profile from `legacy/crates/oino-provider-catalog` (OpenRouter, Groq, DeepSeek, xAI, Together, Fireworks, Cerebras, Mistral, Ollama, LM Studio, OmniRoute, …) | API key (`authEnv` per provider); local providers none |
| `oino.openai-responses` | `extensions/providers/openai-responses` | `openai-responses` | `openai` | API key `OPENAI_API_KEY` |
| `oino.anthropic` | `extensions/providers/anthropic` | `anthropic-messages` | `anthropic` | API key `ANTHROPIC_API_KEY`, OAuth (Claude Pro/Max, pi's flow) via `oauth-login`/`oauth-refresh` |
| `oino.router` | `extensions/providers/router` | `oino-router` | `oino` (Oino Router; per-model Anthropic Messages or OpenAI chat wire) | API key |

Rules:

1. Wire logic ports `legacy/extensions/crates/oino-provider-kit` onto `oino-ai` types (no
   mirror types) and emits `ProviderEvent`s exactly per pi stream rules (start first;
   `*_start/_delta/_end` with `contentIndex`; failures after start end with `error`; usage in
   the final message; `stopReason` mapping per pi-ai provider for that api).
2. Honour `StreamOptions`: `reasoning` via model `thinkingLevelMap` (null = unsupported level),
   `thinkingBudgets`, `maxTokens`, `temperature`, `samplingParams`(+ByThinkingLevel),
   `cacheRetention` and `sessionId` (pi cache hints per api), `toolChoice`, `timeoutMs`,
   `maxRetries`/`maxRetryDelayMs` (provider-level retry, default 0), `headers` (null deletes),
   `metadata`.
3. Call `provider-host.before-request` with the exact JSON payload before sending (enables
   `before_provider_request`/`before_provider_headers`), `after-response` on headers, and
   `stream-event` per raw SSE event when `wantStreamEvents`.
4. `list-models`: static catalog metadata merged with the provider's `/models` endpoint when
   it exists (OpenRouter pricing/context); returns pi `Model` objects (`api` = the component's
   api, `provider` = provider id).
5. Network permission: catalog base-URL hosts (granted automatically, §5.6) plus user
   `baseUrl` overrides from `models.json` (core passes them; host allows the configured host for
   that provider's requests).
6. Acceptance: request-payload goldens — for ≥ 10 contexts per api (text, images, tools,
   thinking with signatures, cache hints, tool results with images), the component's payload
   equals pi-ai 1.4.0's payload for the same model/context (oracle `scripts/pi-oracle/
   providers-v1/payload.ts`), and recorded SSE fixtures parse into the same `AssistantMessage`
   as pi-ai.

## 13. Acceptance (wit-v1)

1. WIT validates; guest SDK compiles both worlds; fixtures build.
2. Host tests: every import (allowed + denied), concurrency (async-probe), deadlines, trap
   restart, cancellation, permissions, preopens, tasks, process tree kill, HTTP allowlist.
3. Core integration (with core-runtime): `echo` fixture sees all 41 pi events with pi payloads
   in a scripted session; results compose as runtime §7.2.
4. Cold start of all bundled components (≈ 25) ≤ 150 ms with a warm compile cache on the dev
   machine (quality §5).
