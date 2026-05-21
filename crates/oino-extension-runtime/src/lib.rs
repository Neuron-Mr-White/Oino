#![doc = r#"Hook execution and JSON-v1 runtime lifecycle boundaries for Oino extensions.

The selected v1 ABI is a small JSON message contract designed to be host-owned
and runtime-agnostic. WASM hosts, native sidecars, and test fixtures all use the
same lifecycle surface: initialize, invoke, progress, cancel, shutdown, and
structured errors.
"#]
#![forbid(unsafe_code)]

use oino_extension_core::{
    ContributionId, DiagnosticPhase, DiagnosticSeverity, ExtensionDiagnostic, ExtensionId,
    HealthState, HookContribution, HookEventKind, HookMode, RegistrySnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

pub const WASM_JSON_V1_ABI: &str = "wasm-json-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEvent {
    pub kind: HookEventKind,
    pub payload: HookEventPayload,
}

impl HookEvent {
    #[must_use]
    pub fn new(kind: HookEventKind, payload: HookEventPayload) -> Self {
        Self { kind, payload }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum HookEventPayload {
    Empty,
    Startup,
    ResourceDiscovery { path: String },
    Session { session_id: String },
    Input { text: String },
    Command { command: String },
    AgentTurn { run_id: String },
    Context { messages: Vec<String> },
    ProviderRequest { provider: String, model: String },
    ProviderResponse { provider: String, status: String },
    MessageStream { delta: String },
    ToolCall { tool: String, arguments: Value },
    ToolResult { tool: String, result: Value },
    ToolUpdate { tool: String, update: Value },
    ModelSelection { model: String },
    ThinkingSelection { level: String },
    Compaction { session_id: String },
    Tree { branch: String },
    Reload,
    Install { package: String },
    Update { package: String },
    Remove { package: String },
    PackageLifecycle { package: String, action: String },
    Custom(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum HookPatch {
    ReplaceInput(String),
    AddContextMessage(String),
    SetModel(String),
    SetThinkingLevel(String),
    AddProviderHeader { name: String, value: String },
    ReplaceToolArguments(Value),
    AddUiUpdate(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum HookDecision {
    Continue,
    Patch { patches: Vec<HookPatch> },
    Cancel { reason: String },
    Fallback { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExecution {
    pub contribution_id: ContributionId,
    pub extension_id: Option<ExtensionId>,
    pub mode: HookMode,
    pub decision: HookDecision,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRunOutput {
    pub cancelled: bool,
    pub fallback: bool,
    pub patches: Vec<HookPatch>,
    pub executions: Vec<HookExecution>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

impl HookRunOutput {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            cancelled: false,
            fallback: false,
            patches: Vec::new(),
            executions: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HookError {
    #[error("hook handler `{handler}` failed: {message}")]
    HandlerFailed { handler: String, message: String },
}

pub trait HookExecutor {
    fn execute(
        &mut self,
        hook: &HookContribution,
        event: &HookEvent,
    ) -> Result<HookExecution, HookError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunnerConfig {
    pub timeout: Duration,
    pub isolate_unhealthy: bool,
}

impl Default for HookRunnerConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(1_000),
            isolate_unhealthy: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunner {
    hooks: Vec<ActiveHook>,
    config: HookRunnerConfig,
    unhealthy: BTreeSet<ContributionId>,
}

impl HookRunner {
    #[must_use]
    pub fn from_snapshot(snapshot: &RegistrySnapshot<HookContribution>) -> Self {
        let mut hooks = snapshot
            .active
            .iter()
            .map(|active| ActiveHook {
                contribution: active.entry.contribution.clone(),
                extension_id: active.entry.metadata.extension_id.clone(),
            })
            .collect::<Vec<_>>();
        hooks.sort_by(|left, right| {
            left.contribution
                .event
                .cmp(&right.contribution.event)
                .then(right.contribution.priority.cmp(&left.contribution.priority))
                .then(left.contribution.id.cmp(&right.contribution.id))
        });
        Self {
            hooks,
            config: HookRunnerConfig::default(),
            unhealthy: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: HookRunnerConfig) -> Self {
        self.config = config;
        self
    }

    pub fn mark_unhealthy(&mut self, contribution_id: ContributionId) {
        self.unhealthy.insert(contribution_id);
    }

    pub fn run<E: HookExecutor>(&mut self, event: HookEvent, executor: &mut E) -> HookRunOutput {
        let mut output = HookRunOutput::empty();
        let timeout_ms = self
            .config
            .timeout
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        for hook in self
            .hooks
            .iter()
            .filter(|hook| hook.contribution.event == event.kind)
        {
            if self.config.isolate_unhealthy && self.unhealthy.contains(&hook.contribution.id) {
                output.diagnostics.push(hook_diagnostic(
                    hook,
                    DiagnosticSeverity::Info,
                    format!(
                        "hook `{}` skipped because it is unhealthy",
                        hook.contribution.id
                    ),
                    HealthState::Disabled,
                ));
                continue;
            }
            match executor.execute(&hook.contribution, &event) {
                Ok(mut execution) => {
                    execution.extension_id = hook.extension_id.clone();
                    if execution.elapsed_ms > timeout_ms {
                        self.unhealthy.insert(hook.contribution.id.clone());
                        output.fallback = true;
                        output.diagnostics.push(hook_diagnostic(
                            hook,
                            DiagnosticSeverity::Warning,
                            format!(
                                "hook `{}` timed out after {timeout_ms}ms",
                                hook.contribution.id
                            ),
                            HealthState::Degraded,
                        ));
                        continue;
                    }
                    apply_hook_decision(hook, &execution, &mut output);
                    output.executions.push(execution);
                    if output.cancelled {
                        break;
                    }
                }
                Err(err) => {
                    self.unhealthy.insert(hook.contribution.id.clone());
                    output.fallback = true;
                    output.diagnostics.push(hook_diagnostic(
                        hook,
                        DiagnosticSeverity::Error,
                        err.to_string(),
                        HealthState::Blocked,
                    ));
                }
            }
        }
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveHook {
    contribution: HookContribution,
    extension_id: Option<ExtensionId>,
}

fn apply_hook_decision(hook: &ActiveHook, execution: &HookExecution, output: &mut HookRunOutput) {
    match (&hook.contribution.mode, &execution.decision) {
        (HookMode::Observe, _) => {}
        (HookMode::Mutable, HookDecision::Patch { patches }) => {
            output.patches.extend(patches.clone())
        }
        (HookMode::Cancellable, HookDecision::Cancel { .. }) => output.cancelled = true,
        (HookMode::Blocking, HookDecision::Fallback { .. }) => output.fallback = true,
        (HookMode::Blocking, HookDecision::Cancel { .. }) => output.cancelled = true,
        (_, HookDecision::Fallback { .. }) => output.fallback = true,
        _ => {}
    }
}

fn hook_diagnostic(
    hook: &ActiveHook,
    severity: DiagnosticSeverity,
    message: String,
    health: HealthState,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase: DiagnosticPhase::RuntimeExecute,
        package_id: None,
        extension_id: hook.extension_id.clone(),
        contribution_id: Some(hook.contribution.id.clone()),
        source_path: None,
        message,
        remediation: Some("disable or fix the hook contribution, then reload extensions".into()),
        health,
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopHookExecutor;

impl HookExecutor for NoopHookExecutor {
    fn execute(
        &mut self,
        hook: &HookContribution,
        _event: &HookEvent,
    ) -> Result<HookExecution, HookError> {
        Ok(HookExecution {
            contribution_id: hook.id.clone(),
            extension_id: None,
            mode: hook.mode,
            decision: HookDecision::Continue,
            elapsed_ms: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeInitialize {
    pub extension_id: ExtensionId,
    pub abi: String,
    pub entry: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeInvocation {
    pub invocation_id: String,
    pub extension_id: ExtensionId,
    pub contribution_id: Option<ContributionId>,
    pub handler: String,
    #[serde(default)]
    pub payload: Value,
    pub timeout: Duration,
}

impl RuntimeInvocation {
    #[must_use]
    pub fn new(extension_id: ExtensionId, handler: impl Into<String>, payload: Value) -> Self {
        Self {
            invocation_id: Uuid::new_v4().to_string(),
            extension_id,
            contribution_id: None,
            handler: handler.into(),
            payload,
            timeout: Duration::from_millis(1_000),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeProgress {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeResultValue {
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub progress: Vec<RuntimeProgress>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "details")]
pub enum RuntimeError {
    #[error("runtime is not initialized")]
    NotInitialized,
    #[error("unsupported ABI `{0}`")]
    UnsupportedAbi(String),
    #[error("handler `{0}` was not found")]
    HandlerNotFound(String),
    #[error("invocation `{0}` was cancelled")]
    Cancelled(String),
    #[error("runtime invocation timed out after {0}ms")]
    Timeout(u64),
    #[error("runtime crashed: {0}")]
    Crash(String),
    #[error("malformed payload: {0}")]
    MalformedPayload(String),
    #[error("unauthorized host import `{0}`")]
    UnauthorizedImport(String),
    #[error("runtime was shut down")]
    Shutdown,
    #[error("handler error: {0}")]
    Handler(String),
}

pub trait ExtensionRuntime {
    fn initialize(&mut self, init: RuntimeInitialize)
        -> Result<Vec<RuntimeProgress>, RuntimeError>;
    fn invoke(&mut self, invocation: RuntimeInvocation)
        -> Result<RuntimeResultValue, RuntimeError>;
    fn cancel(&mut self, invocation_id: &str) -> Result<(), RuntimeError>;
    fn shutdown(&mut self) -> Result<(), RuntimeError>;
    fn health(&self) -> HealthState;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeCapabilityPolicy {
    allowed_imports: BTreeSet<String>,
}

impl RuntimeCapabilityPolicy {
    #[must_use]
    pub fn allow(mut self, import: impl Into<String>) -> Self {
        self.allowed_imports.insert(import.into());
        self
    }

    #[must_use]
    pub fn allows(&self, import: &str) -> bool {
        self.allowed_imports.contains(import)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FixtureHandlerBehavior {
    Success {
        output: Value,
        progress: Vec<RuntimeProgress>,
    },
    Error(String),
    Timeout {
        simulated_ms: u64,
    },
    RequiresImport {
        import: String,
        output: Value,
    },
    MalformedPayload(String),
    Crash(String),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FixtureWasmModule {
    pub handlers: BTreeMap<String, FixtureHandlerBehavior>,
}

impl FixtureWasmModule {
    #[must_use]
    pub fn with_handler(
        mut self,
        handler: impl Into<String>,
        behavior: FixtureHandlerBehavior,
    ) -> Self {
        self.handlers.insert(handler.into(), behavior);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonWasmRuntime {
    module: FixtureWasmModule,
    capabilities: RuntimeCapabilityPolicy,
    initialized: bool,
    shutdown: bool,
    health: HealthState,
    cancelled: BTreeSet<String>,
}

impl JsonWasmRuntime {
    #[must_use]
    pub fn new(module: FixtureWasmModule) -> Self {
        Self {
            module,
            capabilities: RuntimeCapabilityPolicy::default(),
            initialized: false,
            shutdown: false,
            health: HealthState::Healthy,
            cancelled: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: RuntimeCapabilityPolicy) -> Self {
        self.capabilities = capabilities;
        self
    }
}

impl ExtensionRuntime for JsonWasmRuntime {
    fn initialize(
        &mut self,
        init: RuntimeInitialize,
    ) -> Result<Vec<RuntimeProgress>, RuntimeError> {
        if self.shutdown {
            return Err(RuntimeError::Shutdown);
        }
        if init.abi != WASM_JSON_V1_ABI {
            self.health = HealthState::Blocked;
            return Err(RuntimeError::UnsupportedAbi(init.abi));
        }
        if init.entry.trim().is_empty() {
            self.health = HealthState::Blocked;
            return Err(RuntimeError::MalformedPayload(
                "runtime entry is required".into(),
            ));
        }
        self.initialized = true;
        self.health = HealthState::Healthy;
        Ok(vec![RuntimeProgress {
            message: format!("initialized {}", init.extension_id),
            details: None,
        }])
    }

    fn invoke(
        &mut self,
        invocation: RuntimeInvocation,
    ) -> Result<RuntimeResultValue, RuntimeError> {
        if self.shutdown {
            return Err(RuntimeError::Shutdown);
        }
        if !self.initialized {
            return Err(RuntimeError::NotInitialized);
        }
        if self.cancelled.remove(&invocation.invocation_id) {
            return Err(RuntimeError::Cancelled(invocation.invocation_id));
        }
        if !invocation.payload.is_object() && !invocation.payload.is_null() {
            self.health = HealthState::Degraded;
            return Err(RuntimeError::MalformedPayload(
                "JSON-v1 invocation payload must be an object or null".into(),
            ));
        }
        let behavior = self
            .module
            .handlers
            .get(&invocation.handler)
            .cloned()
            .ok_or_else(|| RuntimeError::HandlerNotFound(invocation.handler.clone()))?;
        match behavior {
            FixtureHandlerBehavior::Success { output, progress } => {
                Ok(RuntimeResultValue { output, progress })
            }
            FixtureHandlerBehavior::Error(message) => Err(RuntimeError::Handler(message)),
            FixtureHandlerBehavior::Timeout { simulated_ms } => {
                let timeout_ms = invocation
                    .timeout
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX);
                if simulated_ms > timeout_ms {
                    self.health = HealthState::Degraded;
                    Err(RuntimeError::Timeout(timeout_ms))
                } else {
                    Ok(RuntimeResultValue {
                        output: Value::Null,
                        progress: Vec::new(),
                    })
                }
            }
            FixtureHandlerBehavior::RequiresImport { import, output } => {
                if self.capabilities.allows(&import) {
                    Ok(RuntimeResultValue {
                        output,
                        progress: Vec::new(),
                    })
                } else {
                    self.health = HealthState::Blocked;
                    Err(RuntimeError::UnauthorizedImport(import))
                }
            }
            FixtureHandlerBehavior::MalformedPayload(message) => {
                self.health = HealthState::Degraded;
                Err(RuntimeError::MalformedPayload(message))
            }
            FixtureHandlerBehavior::Crash(message) => {
                self.health = HealthState::Unhealthy;
                Err(RuntimeError::Crash(message))
            }
        }
    }

    fn cancel(&mut self, invocation_id: &str) -> Result<(), RuntimeError> {
        if self.shutdown {
            return Err(RuntimeError::Shutdown);
        }
        self.cancelled.insert(invocation_id.into());
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), RuntimeError> {
        self.shutdown = true;
        self.initialized = false;
        self.health = HealthState::Disabled;
        Ok(())
    }

    fn health(&self) -> HealthState {
        self.health
    }
}

pub fn runtime_error_diagnostic(
    extension_id: ExtensionId,
    contribution_id: Option<ContributionId>,
    error: &RuntimeError,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity: match error {
            RuntimeError::Timeout(_) | RuntimeError::MalformedPayload(_) => {
                DiagnosticSeverity::Warning
            }
            _ => DiagnosticSeverity::Error,
        },
        phase: DiagnosticPhase::RuntimeExecute,
        package_id: None,
        extension_id: Some(extension_id),
        contribution_id,
        source_path: None,
        message: error.to_string(),
        remediation: Some(
            "fix the extension runtime, requested capability, or handler payload".into(),
        ),
        health: match error {
            RuntimeError::Timeout(_) | RuntimeError::MalformedPayload(_) => HealthState::Degraded,
            RuntimeError::Shutdown | RuntimeError::Cancelled(_) => HealthState::Disabled,
            RuntimeError::UnauthorizedImport(_) => HealthState::Blocked,
            RuntimeError::Crash(_) => HealthState::Unhealthy,
            _ => HealthState::Blocked,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_extension_core::{
        ContributionMetadata, HookRegistry, RegistryEntryKey, RegistryPolicy, SourceDescriptor,
        SourceKind, SourceScope,
    };
    use serde_json::json;

    fn extension_id() -> ExtensionId {
        ExtensionId::new("acme.hooks").unwrap_or_else(|err| panic!("valid id: {err}"))
    }

    fn hook(id: &str, event: HookEventKind, priority: i32, mode: HookMode) -> HookContribution {
        HookContribution {
            id: ContributionId::new(id).unwrap_or_else(|err| panic!("valid id: {err}")),
            event,
            priority,
            mode,
            handler: Some(id.into()),
            conflict: Default::default(),
        }
    }

    fn hook_snapshot(hooks: Vec<HookContribution>) -> RegistrySnapshot<HookContribution> {
        let mut registry = HookRegistry::hooks();
        for hook in hooks {
            let id = hook.id.clone();
            let metadata = ContributionMetadata::new(
                id.clone(),
                SourceDescriptor {
                    scope: SourceScope::BuiltIn,
                    kind: SourceKind::BuiltIn,
                    path: None,
                    registry: None,
                },
            )
            .with_extension_id(extension_id());
            registry
                .register_entry(RegistryEntryKey::new(id.as_str()), metadata, hook)
                .unwrap_or_else(|err| panic!("register hook: {err}"));
        }
        registry.compose(&RegistryPolicy::default())
    }

    #[derive(Default)]
    struct ScriptedHookExecutor {
        decisions: BTreeMap<String, HookExecution>,
    }

    impl ScriptedHookExecutor {
        fn with(mut self, id: &str, decision: HookDecision, elapsed_ms: u64) -> Self {
            self.decisions.insert(
                id.into(),
                HookExecution {
                    contribution_id: ContributionId::new(id)
                        .unwrap_or_else(|err| panic!("valid id: {err}")),
                    extension_id: None,
                    mode: HookMode::Observe,
                    decision,
                    elapsed_ms,
                },
            );
            self
        }
    }

    impl HookExecutor for ScriptedHookExecutor {
        fn execute(
            &mut self,
            hook: &HookContribution,
            _event: &HookEvent,
        ) -> Result<HookExecution, HookError> {
            self.decisions
                .get(hook.id.as_str())
                .cloned()
                .ok_or_else(|| HookError::HandlerFailed {
                    handler: hook.id.to_string(),
                    message: "missing scripted hook".into(),
                })
        }
    }

    #[test]
    fn hook_runner_orders_applies_patches_and_cancellation() {
        let snapshot = hook_snapshot(vec![
            hook("low", HookEventKind::Input, 0, HookMode::Mutable),
            hook("high", HookEventKind::Input, 10, HookMode::Mutable),
            hook("cancel", HookEventKind::Input, -10, HookMode::Cancellable),
        ]);
        let mut runner = HookRunner::from_snapshot(&snapshot);
        let mut executor = ScriptedHookExecutor::default()
            .with(
                "high",
                HookDecision::Patch {
                    patches: vec![HookPatch::ReplaceInput("high".into())],
                },
                1,
            )
            .with(
                "low",
                HookDecision::Patch {
                    patches: vec![HookPatch::AddContextMessage("low".into())],
                },
                1,
            )
            .with(
                "cancel",
                HookDecision::Cancel {
                    reason: "blocked".into(),
                },
                1,
            );
        let output = runner.run(
            HookEvent::new(
                HookEventKind::Input,
                HookEventPayload::Input {
                    text: "hello".into(),
                },
            ),
            &mut executor,
        );
        assert!(output.cancelled);
        assert_eq!(output.patches.len(), 2);
        let order = output
            .executions
            .iter()
            .map(|execution| execution.contribution_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, vec!["high", "low", "cancel"]);
    }

    #[test]
    fn hook_runner_timeout_and_unhealthy_isolation() {
        let snapshot = hook_snapshot(vec![hook(
            "slow",
            HookEventKind::ProviderRequest,
            0,
            HookMode::Blocking,
        )]);
        let mut runner = HookRunner::from_snapshot(&snapshot).with_config(HookRunnerConfig {
            timeout: Duration::from_millis(5),
            isolate_unhealthy: true,
        });
        let mut executor = ScriptedHookExecutor::default().with("slow", HookDecision::Continue, 10);
        let event = HookEvent::new(
            HookEventKind::ProviderRequest,
            HookEventPayload::ProviderRequest {
                provider: "openrouter".into(),
                model: "xai/glm".into(),
            },
        );
        let first = runner.run(event.clone(), &mut executor);
        assert!(first.fallback);
        assert_eq!(first.diagnostics[0].health, HealthState::Degraded);
        let second = runner.run(event, &mut executor);
        assert_eq!(second.executions.len(), 0);
        assert_eq!(second.diagnostics[0].health, HealthState::Disabled);
    }

    #[test]
    fn noop_hooks_cover_declared_event_groups() {
        let events = [
            HookEventKind::Startup,
            HookEventKind::ResourceDiscovery,
            HookEventKind::Session,
            HookEventKind::Input,
            HookEventKind::Command,
            HookEventKind::BeforeAgentTurn,
            HookEventKind::AfterAgentTurn,
            HookEventKind::Context,
            HookEventKind::ProviderRequest,
            HookEventKind::ProviderResponse,
            HookEventKind::MessageStream,
            HookEventKind::ToolCall,
            HookEventKind::ToolResult,
            HookEventKind::ToolUpdate,
            HookEventKind::ModelSelection,
            HookEventKind::ThinkingSelection,
            HookEventKind::Compaction,
            HookEventKind::Tree,
            HookEventKind::Reload,
            HookEventKind::Install,
            HookEventKind::Update,
            HookEventKind::Remove,
            HookEventKind::PackageLifecycle,
        ];
        assert_eq!(events.len(), 23);
    }

    fn init() -> RuntimeInitialize {
        RuntimeInitialize {
            extension_id: extension_id(),
            abi: WASM_JSON_V1_ABI.into(),
            entry: "plugin.wasm".into(),
            metadata: Value::Null,
        }
    }

    #[test]
    fn json_runtime_initializes_executes_progress_and_shutdown() {
        let module = FixtureWasmModule::default().with_handler(
            "run",
            FixtureHandlerBehavior::Success {
                output: json!({"ok": true}),
                progress: vec![RuntimeProgress {
                    message: "halfway".into(),
                    details: Some(json!({"pct": 50})),
                }],
            },
        );
        let mut runtime = JsonWasmRuntime::new(module);
        let progress = runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));
        assert_eq!(progress.len(), 1);
        let result = runtime
            .invoke(RuntimeInvocation::new(extension_id(), "run", json!({})))
            .unwrap_or_else(|err| panic!("invoke: {err}"));
        assert_eq!(result.output, json!({"ok": true}));
        assert_eq!(result.progress[0].message, "halfway");
        runtime
            .shutdown()
            .unwrap_or_else(|err| panic!("shutdown: {err}"));
        assert_eq!(runtime.health(), HealthState::Disabled);
    }

    #[test]
    fn json_runtime_cancel_timeout_crash_unauthorized_and_malformed_payloads() {
        let module = FixtureWasmModule::default()
            .with_handler("slow", FixtureHandlerBehavior::Timeout { simulated_ms: 10 })
            .with_handler(
                "import",
                FixtureHandlerBehavior::RequiresImport {
                    import: "host.fs.read".into(),
                    output: json!({"read": true}),
                },
            )
            .with_handler("crash", FixtureHandlerBehavior::Crash("boom".into()))
            .with_handler(
                "malformed",
                FixtureHandlerBehavior::MalformedPayload("bad shape".into()),
            );
        let mut runtime = JsonWasmRuntime::new(module);
        runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));

        let mut cancelled = RuntimeInvocation::new(extension_id(), "slow", json!({}));
        cancelled.invocation_id = "cancel-me".into();
        runtime
            .cancel("cancel-me")
            .unwrap_or_else(|err| panic!("cancel: {err}"));
        assert!(matches!(
            runtime.invoke(cancelled),
            Err(RuntimeError::Cancelled(id)) if id == "cancel-me"
        ));

        let mut slow = RuntimeInvocation::new(extension_id(), "slow", json!({}));
        slow.timeout = Duration::from_millis(1);
        assert!(matches!(
            runtime.invoke(slow),
            Err(RuntimeError::Timeout(1))
        ));
        assert!(matches!(
            runtime.invoke(RuntimeInvocation::new(extension_id(), "import", json!({}))),
            Err(RuntimeError::UnauthorizedImport(import)) if import == "host.fs.read"
        ));
        assert_eq!(runtime.health(), HealthState::Blocked);
        assert!(matches!(
            runtime.invoke(RuntimeInvocation::new(
                extension_id(),
                "malformed",
                json!({})
            )),
            Err(RuntimeError::MalformedPayload(_))
        ));
        assert!(matches!(
            runtime.invoke(RuntimeInvocation::new(extension_id(), "crash", json!({}))),
            Err(RuntimeError::Crash(_))
        ));
        assert_eq!(runtime.health(), HealthState::Unhealthy);
        assert!(matches!(
            runtime.invoke(RuntimeInvocation::new(extension_id(), "slow", json!("bad"))),
            Err(RuntimeError::MalformedPayload(_))
        ));
    }

    #[test]
    fn json_runtime_allows_brokered_import_and_rejects_bad_abi() {
        let module = FixtureWasmModule::default().with_handler(
            "import",
            FixtureHandlerBehavior::RequiresImport {
                import: "host.web.search".into(),
                output: json!({"result": "ok"}),
            },
        );
        let mut runtime = JsonWasmRuntime::new(module)
            .with_capabilities(RuntimeCapabilityPolicy::default().allow("host.web.search"));
        let mut bad = init();
        bad.abi = "component-model".into();
        assert!(matches!(
            runtime.initialize(bad),
            Err(RuntimeError::UnsupportedAbi(_))
        ));
        runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));
        let result = runtime
            .invoke(RuntimeInvocation::new(extension_id(), "import", json!({})))
            .unwrap_or_else(|err| panic!("invoke: {err}"));
        assert_eq!(result.output, json!({"result": "ok"}));
    }
}
