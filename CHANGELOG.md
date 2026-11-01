# Changelog

Notable changes to the published `oino-*` crates. The application (`apps/`) tracks the
workspace version and is released separately.

## 2026-11-01 — host-embedding fixes (fd budget + open latency)

### oino-client 1.1.0

Fixes for session hosts that embed `SessionHub` (descriptor exhaustion, first-open
latency, refused opens). Additive API only — no breaking changes.

- Added `SessionHub::create_with(opts, limits)` and the new `HubLimits` type
  (`#[non_exhaustive]`, `Default`, builder): host-level limits layered over the
  `extensionSettings.hub.*` settings. First knob: `HubLimits::default()
  .with_max_services(n)` caps the services LRU at `n` sets — an embedding host sets it
  from its own file-descriptor budget, because every cached services set holds one wasm
  instance per extension (preopen directories + log files) and the default of 4 sets
  can exceed a GUI-launched host's `RLIMIT_NOFILE`. `SessionHub::create` is unchanged.
- Added `SessionHub::warm_services(&cwd)`: pre-warms (or returns) the cached services of
  a project directory with no session runtime, so a later `session_open` there does not
  pay services creation (extension discovery, wasm instances) on the click path.
- Added `SessionHub::max_services()`: the effective cap of this hub (setting or
  `HubLimits` override).
- The hub no longer refuses an open when its services cache is full: it evicts the
  least-recently-used **idle** services set (its idle sessions return to records and
  re-wake on demand, exactly like the idle monitor's eviction) and, when every cached
  set has a busy session (running turn, attached stream, open dialog), exceeds the cap
  by one with a warning instead of erroring. Only an open past that bound is refused.
- Note for a future 2.0: `HubOptions` itself stays a plain (exhaustively-constructible)
  struct literal; host knobs live in `HubLimits` so they can grow additively.

### oino-core 1.0.1

- `Desktop`/`Mobile` service builds no longer wait on provider `list-models` probes at
  startup: the dynamic-model refresh now runs in the background and lands in the shared
  `ModelRegistry` as it completes. Terminal (CLI/RPC) modes keep awaiting it, so their
  startup behavior is unchanged. This removes multi-second stalls on every services
  build (first session open in a project) when local model servers such as ollama or
  lmstudio are unreachable.
- No public API changes.

### oino-extension-api 1.1.0

- `HostService` (the trait hosts implement for `host-service.call`) gains a **defaulted**
  method `call_in(&self, cx: &CallContext, op, payload, cancel)` — [`HostService::call`]
  with the caller's context (session id, cwd, …), for services that keep per-session
  state (e.g. a browser service scoping its tabs by session). The default ignores `cx`
  and delegates to `call`, so existing implementations and callers are unaffected;
  hosts that need the context override `call_in`.
- No other changes; no breaking changes.

### oino-wasm-host 1.0.1

- One log descriptor per wasm instance instead of two: an extension's stdout and stderr
  now append through a single shared descriptor (`O_APPEND` keeps the interleaved
  writes atomic), saving one file descriptor per extension instance — about 31 per
  services set on a full install, which mattered for hosts near their
  `RLIMIT_NOFILE` budget.
- The hub forwards host-service calls through `HostService::call_in`
  (oino-extension-api 1.1.0), passing the caller's `CallContext`, so per-session host
  services scope their state correctly; services implementing only `call` behave
  exactly as before. Requires `oino-extension-api ^1.1.0`.
- No public API changes.
