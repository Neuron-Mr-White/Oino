#![doc = r#"Hook execution and JSON-v1 runtime lifecycle boundaries for Oino extensions.

The selected v1 ABI is a small JSON message contract designed to be host-owned
and runtime-agnostic. WASM hosts, native sidecars, and test fixtures all use the
same lifecycle surface: initialize, invoke, progress, cancel, shutdown, and
structured errors.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use oino_agent_loop::{
    AbortSignal, LoopError, LoopResult, Tool, ToolCall, ToolDefinition,
    ToolExecutionMode as AgentToolExecutionMode, ToolResult, ToolUpdate, ToolUpdateCallback,
};
use oino_extension_core::{
    ContributionId, DiagnosticPhase, DiagnosticSeverity, ExtensionDiagnostic, ExtensionId,
    ExtensionPermissions, HealthState, HookContribution, HookEventKind, HookMode, RegistrySnapshot,
    ToolContribution, ToolExecutionMode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
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

pub trait ExtensionRuntime: Send {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityDecision {
    Allowed,
    Denied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityAudit {
    pub request_id: String,
    pub extension_id: ExtensionId,
    pub contribution_id: Option<ContributionId>,
    pub capability: String,
    pub decision: CapabilityDecision,
    pub timeout_ms: u64,
    pub payload_bytes: usize,
    pub response_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub request_id: String,
    pub extension_id: ExtensionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contribution_id: Option<ContributionId>,
    pub capability: String,
    #[serde(default)]
    pub payload: Value,
    pub timeout: Duration,
    pub max_response_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

impl CapabilityRequest {
    #[must_use]
    pub fn new(extension_id: ExtensionId, capability: impl Into<String>, payload: Value) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            extension_id,
            contribution_id: None,
            capability: capability.into(),
            payload,
            timeout: Duration::from_millis(1_000),
            max_response_bytes: 16 * 1024,
            provenance: None,
        }
    }

    #[must_use]
    pub fn with_contribution_id(mut self, contribution_id: ContributionId) -> Self {
        self.contribution_id = Some(contribution_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityResponse {
    #[serde(default)]
    pub output: Value,
    pub audit: CapabilityAudit,
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "details")]
pub enum CapabilityError {
    #[error("capability `{capability}` denied: {reason}")]
    PermissionDenied { capability: String, reason: String },
    #[error("capability `{0}` is unknown")]
    UnknownCapability(String),
    #[error("capability request timed out after {0}ms")]
    Timeout(u64),
    #[error("invalid capability payload: {0}")]
    InvalidPayload(String),
    #[error("capability payload is {actual} bytes, max is {max}")]
    OversizedPayload { actual: usize, max: usize },
    #[error("capability response is {actual} bytes, max is {max}")]
    OversizedResponse { actual: usize, max: usize },
    #[error("extension `{0}` is unhealthy")]
    UnhealthyExtension(ExtensionId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinCapability {
    Echo,
    MockWebSearch,
    PersistenceRead,
    PersistenceWrite,
    PersistenceDelete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityBroker {
    permissions: BTreeMap<ExtensionId, ExtensionPermissions>,
    capabilities: BTreeMap<String, BuiltinCapability>,
    unhealthy: BTreeSet<ExtensionId>,
    max_payload_bytes: usize,
    audits: Vec<CapabilityAudit>,
}

impl Default for CapabilityBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityBroker {
    #[must_use]
    pub fn new() -> Self {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("host.test.echo".into(), BuiltinCapability::Echo);
        capabilities.insert("host.web.search".into(), BuiltinCapability::MockWebSearch);
        capabilities.insert(
            "host.persistence.read".into(),
            BuiltinCapability::PersistenceRead,
        );
        capabilities.insert(
            "host.persistence.write".into(),
            BuiltinCapability::PersistenceWrite,
        );
        capabilities.insert(
            "host.persistence.delete".into(),
            BuiltinCapability::PersistenceDelete,
        );
        Self {
            permissions: BTreeMap::new(),
            capabilities,
            unhealthy: BTreeSet::new(),
            max_payload_bytes: 16 * 1024,
            audits: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_max_payload_bytes(mut self, max_payload_bytes: usize) -> Self {
        self.max_payload_bytes = max_payload_bytes;
        self
    }

    pub fn register_permissions(
        &mut self,
        extension_id: ExtensionId,
        permissions: ExtensionPermissions,
    ) {
        self.permissions.insert(extension_id, permissions);
    }

    pub fn mark_unhealthy(&mut self, extension_id: ExtensionId) {
        self.unhealthy.insert(extension_id);
    }

    #[must_use]
    pub fn audits(&self) -> &[CapabilityAudit] {
        &self.audits
    }

    pub fn call(
        &mut self,
        request: CapabilityRequest,
    ) -> Result<CapabilityResponse, CapabilityError> {
        let payload_bytes = json_size(&request.payload)?;
        if payload_bytes > self.max_payload_bytes {
            let err = CapabilityError::OversizedPayload {
                actual: payload_bytes,
                max: self.max_payload_bytes,
            };
            self.audit_error(&request, payload_bytes, err.clone());
            return Err(err);
        }
        if self.unhealthy.contains(&request.extension_id) {
            let err = CapabilityError::UnhealthyExtension(request.extension_id.clone());
            self.audit_error(&request, payload_bytes, err.clone());
            return Err(err);
        }
        let permissions = self.permissions.get(&request.extension_id);
        if !permissions
            .is_some_and(|permissions| permissions.allows_host_capability(&request.capability))
        {
            let err = CapabilityError::PermissionDenied {
                capability: request.capability.clone(),
                reason: "extension manifest or policy did not grant this host capability".into(),
            };
            self.audit_error(&request, payload_bytes, err.clone());
            return Err(err);
        }
        let Some(capability) = self.capabilities.get(&request.capability).cloned() else {
            let err = CapabilityError::UnknownCapability(request.capability.clone());
            self.audit_error(&request, payload_bytes, err.clone());
            return Err(err);
        };
        let timeout_ms = request.timeout.as_millis().try_into().unwrap_or(u64::MAX);
        let output = match execute_capability(capability, &request.payload, timeout_ms) {
            Ok(output) => output,
            Err(err) => {
                self.audit_error(&request, payload_bytes, err.clone());
                return Err(err);
            }
        };
        let response_bytes = json_size(&output)?;
        if response_bytes > request.max_response_bytes {
            let err = CapabilityError::OversizedResponse {
                actual: response_bytes,
                max: request.max_response_bytes,
            };
            self.audit_error(&request, payload_bytes, err.clone());
            return Err(err);
        }
        let audit = CapabilityAudit {
            request_id: request.request_id,
            extension_id: request.extension_id,
            contribution_id: request.contribution_id,
            capability: request.capability,
            decision: CapabilityDecision::Allowed,
            timeout_ms,
            payload_bytes,
            response_bytes: Some(response_bytes),
            provenance: request.provenance,
        };
        self.audits.push(audit.clone());
        Ok(CapabilityResponse { output, audit })
    }

    fn audit_error(
        &mut self,
        request: &CapabilityRequest,
        payload_bytes: usize,
        error: CapabilityError,
    ) {
        self.audits.push(CapabilityAudit {
            request_id: request.request_id.clone(),
            extension_id: request.extension_id.clone(),
            contribution_id: request.contribution_id.clone(),
            capability: request.capability.clone(),
            decision: CapabilityDecision::Denied {
                reason: error.to_string(),
            },
            timeout_ms: request.timeout.as_millis().try_into().unwrap_or(u64::MAX),
            payload_bytes,
            response_bytes: None,
            provenance: request.provenance.clone(),
        });
    }
}

fn execute_capability(
    capability: BuiltinCapability,
    payload: &Value,
    timeout_ms: u64,
) -> Result<Value, CapabilityError> {
    if timeout_ms == 0 {
        return Err(CapabilityError::Timeout(0));
    }
    let simulated_ms = payload
        .get("simulate_timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if simulated_ms > timeout_ms {
        return Err(CapabilityError::Timeout(timeout_ms));
    }
    match capability {
        BuiltinCapability::Echo => Ok(payload.clone()),
        BuiltinCapability::MockWebSearch => {
            let query = payload
                .get("query")
                .and_then(Value::as_str)
                .filter(|query| !query.trim().is_empty())
                .ok_or_else(|| CapabilityError::InvalidPayload("query is required".into()))?;
            Ok(serde_json::json!({
                "query": query,
                "results": [{
                    "title": "Mock Oino result",
                    "url": "https://example.invalid/oino/mock-search",
                    "snippet": format!("mock result for {query}")
                }]
            }))
        }
        BuiltinCapability::PersistenceRead => {
            let key = persistence_key(payload)?;
            Ok(serde_json::json!({ "key": key, "found": false, "value": null }))
        }
        BuiltinCapability::PersistenceWrite => {
            let key = persistence_key(payload)?;
            let value = payload
                .get("value")
                .ok_or_else(|| CapabilityError::InvalidPayload("value is required".into()))?;
            Ok(serde_json::json!({ "key": key, "written": true, "bytes": json_size(value)? }))
        }
        BuiltinCapability::PersistenceDelete => {
            let key = persistence_key(payload)?;
            Ok(serde_json::json!({ "key": key, "deleted": true }))
        }
    }
}

fn persistence_key(payload: &Value) -> Result<&str, CapabilityError> {
    let key = payload
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| {
            !key.trim().is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
        .ok_or_else(|| CapabilityError::InvalidPayload("valid key is required".into()))?;
    let scope = payload
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("session");
    if !matches!(scope, "session" | "project" | "global") {
        return Err(CapabilityError::InvalidPayload(
            "scope must be session, project, or global".into(),
        ));
    }
    Ok(key)
}

fn json_size(value: &Value) -> Result<usize, CapabilityError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|err| CapabilityError::InvalidPayload(err.to_string()))
}

pub fn capability_error_diagnostic(
    request: &CapabilityRequest,
    error: &CapabilityError,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity: match error {
            CapabilityError::PermissionDenied { .. }
            | CapabilityError::UnhealthyExtension(_)
            | CapabilityError::UnknownCapability(_) => DiagnosticSeverity::Error,
            CapabilityError::Timeout(_)
            | CapabilityError::InvalidPayload(_)
            | CapabilityError::OversizedPayload { .. }
            | CapabilityError::OversizedResponse { .. } => DiagnosticSeverity::Warning,
        },
        phase: DiagnosticPhase::Permission,
        package_id: None,
        extension_id: Some(request.extension_id.clone()),
        contribution_id: request.contribution_id.clone(),
        source_path: None,
        message: error.to_string(),
        remediation: Some(
            "update extension permissions, reduce payload size, or fix the capability call".into(),
        ),
        health: match error {
            CapabilityError::PermissionDenied { .. } | CapabilityError::UnknownCapability(_) => {
                HealthState::Blocked
            }
            CapabilityError::UnhealthyExtension(_) => HealthState::Unhealthy,
            CapabilityError::Timeout(_)
            | CapabilityError::InvalidPayload(_)
            | CapabilityError::OversizedPayload { .. }
            | CapabilityError::OversizedResponse { .. } => HealthState::Degraded,
        },
    }
}

#[derive(Clone)]
pub struct ExtensionToolAdapter {
    contribution: ToolContribution,
    extension_id: ExtensionId,
    runtime: SharedRuntime,
}

pub type SharedRuntime = Arc<Mutex<Box<dyn ExtensionRuntime + Send>>>;

impl ExtensionToolAdapter {
    #[must_use]
    pub fn new(
        contribution: ToolContribution,
        extension_id: ExtensionId,
        runtime: SharedRuntime,
    ) -> Self {
        Self {
            contribution,
            extension_id,
            runtime,
        }
    }
}

#[async_trait]
impl Tool for ExtensionToolAdapter {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.contribution.id.to_string(),
            description: self.contribution.description.clone(),
            input_schema: self.contribution.input_schema.clone(),
        }
    }

    fn execution_mode(&self) -> AgentToolExecutionMode {
        match self.contribution.execution_mode {
            ToolExecutionMode::Parallel => AgentToolExecutionMode::Parallel,
            ToolExecutionMode::Sequential => AgentToolExecutionMode::Sequential,
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        if signal.is_aborted() {
            return Err(LoopError::Tool(
                "extension tool cancelled before start".into(),
            ));
        }
        let handler = self
            .contribution
            .handler
            .clone()
            .unwrap_or_else(|| self.contribution.id.to_string());
        let mut invocation =
            RuntimeInvocation::new(self.extension_id.clone(), handler, call.arguments.clone());
        invocation.contribution_id = Some(self.contribution.id.clone());
        invocation.invocation_id = call.id.to_string();
        let result = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| LoopError::Tool("extension runtime lock poisoned".into()))?;
            runtime.invoke(invocation)
        };
        if signal.is_aborted() {
            if let Ok(mut runtime) = self.runtime.lock() {
                let _ = runtime.cancel(&call.id.to_string());
            }
            return Err(LoopError::Tool("extension tool cancelled".into()));
        }
        match result {
            Ok(value) => {
                for progress in &value.progress {
                    updates
                        .update(ToolUpdate {
                            message: progress.message.clone(),
                            details: progress.details.clone(),
                        })
                        .await?;
                }
                Ok(ToolResult::text(&call, value.output.to_string()))
            }
            Err(err) => Ok(ToolResult::error(&call, err.to_string())),
        }
    }
}

#[derive(Clone)]
pub struct ExtensionCommandAdapter {
    contribution_id: ContributionId,
    description: String,
    extension_id: ExtensionId,
    handler: String,
    runtime: SharedRuntime,
}

impl ExtensionCommandAdapter {
    #[must_use]
    pub fn new(
        contribution_id: ContributionId,
        description: impl Into<String>,
        extension_id: ExtensionId,
        handler: impl Into<String>,
        runtime: SharedRuntime,
    ) -> Self {
        Self {
            contribution_id,
            description: description.into(),
            extension_id,
            handler: handler.into(),
            runtime,
        }
    }

    #[must_use]
    pub fn contribution_id(&self) -> &ContributionId {
        &self.contribution_id
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn execute(&self, arguments: Value) -> Result<Value, RuntimeError> {
        let mut invocation =
            RuntimeInvocation::new(self.extension_id.clone(), self.handler.clone(), arguments);
        invocation.contribution_id = Some(self.contribution_id.clone());
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| RuntimeError::Crash("extension runtime lock poisoned".into()))?;
        runtime.invoke(invocation).map(|result| result.output)
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

    fn permissions_with(capability: &str) -> ExtensionPermissions {
        let mut permissions = ExtensionPermissions::default();
        permissions.host_capabilities.insert(capability.into());
        permissions
    }

    #[test]
    fn capability_broker_gates_persistence_capabilities() {
        let mut broker = CapabilityBroker::new();
        broker.register_permissions(extension_id(), permissions_with("host.persistence.write"));
        let response = broker
            .call(CapabilityRequest::new(
                extension_id(),
                "host.persistence.write",
                serde_json::json!({
                    "scope": "project",
                    "key": "processes",
                    "value": { "running": 1 }
                }),
            ))
            .unwrap_or_else(|err| panic!("capability failed: {err}"));
        assert_eq!(response.output["written"], true);
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.persistence.read",
                serde_json::json!({ "scope": "project", "key": "processes" }),
            )),
            Err(CapabilityError::PermissionDenied { .. })
        ));
        broker.register_permissions(extension_id(), permissions_with("host.persistence.read"));
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.persistence.read",
                serde_json::json!({ "scope": "invalid", "key": "processes" }),
            )),
            Err(CapabilityError::InvalidPayload(_))
        ));
    }

    #[test]
    fn capability_broker_allows_mock_capabilities_and_audits() {
        let mut broker = CapabilityBroker::new();
        broker.register_permissions(extension_id(), permissions_with("host.web.search"));
        let response = broker
            .call(CapabilityRequest::new(
                extension_id(),
                "host.web.search",
                json!({"query": "oino"}),
            ))
            .unwrap_or_else(|err| panic!("capability call: {err}"));
        assert_eq!(response.audit.decision, CapabilityDecision::Allowed);
        assert_eq!(response.output["query"], "oino");
        assert_eq!(broker.audits().len(), 1);
    }

    #[test]
    fn capability_broker_denies_times_out_invalid_oversized_and_unhealthy() {
        let mut broker = CapabilityBroker::new().with_max_payload_bytes(20);
        broker.register_permissions(extension_id(), permissions_with("host.web.search"));
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.test.echo",
                json!({"ok": true}),
            )),
            Err(CapabilityError::PermissionDenied { .. })
        ));
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.web.search",
                json!({"query": "oino", "simulate_timeout_ms": 2}),
            )),
            Err(CapabilityError::OversizedPayload { .. })
        ));

        let mut broker = CapabilityBroker::new();
        broker.register_permissions(extension_id(), permissions_with("host.web.search"));
        let mut timeout = CapabilityRequest::new(
            extension_id(),
            "host.web.search",
            json!({"query": "oino", "simulate_timeout_ms": 2}),
        );
        timeout.timeout = Duration::from_millis(1);
        assert!(matches!(
            broker.call(timeout),
            Err(CapabilityError::Timeout(1))
        ));
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.web.search",
                json!({"missing": "query"}),
            )),
            Err(CapabilityError::InvalidPayload(_))
        ));
        broker.mark_unhealthy(extension_id());
        assert!(matches!(
            broker.call(CapabilityRequest::new(
                extension_id(),
                "host.web.search",
                json!({"query": "oino"}),
            )),
            Err(CapabilityError::UnhealthyExtension(_))
        ));
        assert!(broker
            .audits()
            .iter()
            .any(|audit| matches!(audit.decision, CapabilityDecision::Denied { .. })));
    }

    #[tokio::test]
    async fn extension_tool_adapter_returns_normal_tool_results() {
        let module = FixtureWasmModule::default().with_handler(
            "tool.run",
            FixtureHandlerBehavior::Success {
                output: json!({"ok": true}),
                progress: vec![RuntimeProgress {
                    message: "working".into(),
                    details: None,
                }],
            },
        );
        let mut runtime = JsonWasmRuntime::new(module);
        runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));
        let runtime: SharedRuntime = Arc::new(Mutex::new(Box::new(runtime)));
        let adapter = ExtensionToolAdapter::new(
            ToolContribution {
                id: ContributionId::new("visible_tool")
                    .unwrap_or_else(|err| panic!("valid id: {err}")),
                description: "Visible extension tool".into(),
                input_schema: json!({"type": "object"}),
                execution_mode: ToolExecutionMode::Parallel,
                handler: Some("tool.run".into()),
                conflict: Default::default(),
            },
            extension_id(),
            runtime,
        );
        let call = oino_agent_loop::ToolCall {
            id: oino_types::OinoId::nil(),
            name: "visible_tool".into(),
            arguments: json!({}),
        };
        let sink = Arc::new(oino_agent_loop::VecEventSink::new());
        let result = adapter
            .execute(
                call,
                oino_agent_loop::ToolUpdateCallback::new(sink, oino_types::OinoId::nil()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("execute: {err}"));
        assert!(!result.is_error);
        assert!(
            matches!(&result.content[0], oino_types::ContentBlock::Text { text } if text.contains("ok"))
        );
    }

    #[tokio::test]
    async fn extension_tool_adapter_returns_errors_and_honors_cancellation() {
        let module = FixtureWasmModule::default()
            .with_handler("tool.fail", FixtureHandlerBehavior::Error("boom".into()));
        let mut runtime = JsonWasmRuntime::new(module);
        runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));
        let adapter = ExtensionToolAdapter::new(
            ToolContribution {
                id: ContributionId::new("failing_tool")
                    .unwrap_or_else(|err| panic!("valid id: {err}")),
                description: "Failing extension tool".into(),
                input_schema: json!({"type": "object"}),
                execution_mode: ToolExecutionMode::Parallel,
                handler: Some("tool.fail".into()),
                conflict: Default::default(),
            },
            extension_id(),
            Arc::new(Mutex::new(Box::new(runtime))),
        );
        let call = oino_agent_loop::ToolCall {
            id: oino_types::OinoId::nil(),
            name: "failing_tool".into(),
            arguments: json!({}),
        };
        let sink = Arc::new(oino_agent_loop::VecEventSink::new());
        let result = adapter
            .execute(
                call.clone(),
                oino_agent_loop::ToolUpdateCallback::new(
                    Arc::clone(&sink) as Arc<dyn oino_agent_loop::EventSink>,
                    oino_types::OinoId::nil(),
                ),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("execute: {err}"));
        assert!(result.is_error);

        let signal = AbortSignal::new();
        signal.abort();
        assert!(adapter
            .execute(
                call,
                oino_agent_loop::ToolUpdateCallback::new(sink, oino_types::OinoId::nil()),
                signal,
            )
            .await
            .is_err());
    }

    #[test]
    fn extension_command_adapter_routes_success_and_errors() {
        let module = FixtureWasmModule::default()
            .with_handler(
                "cmd.run",
                FixtureHandlerBehavior::Success {
                    output: json!({"message": "done"}),
                    progress: Vec::new(),
                },
            )
            .with_handler("cmd.err", FixtureHandlerBehavior::Error("nope".into()));
        let mut runtime = JsonWasmRuntime::new(module);
        runtime
            .initialize(init())
            .unwrap_or_else(|err| panic!("init: {err}"));
        let runtime: SharedRuntime = Arc::new(Mutex::new(Box::new(runtime)));
        let command = ExtensionCommandAdapter::new(
            ContributionId::new("demo_command").unwrap_or_else(|err| panic!("valid id: {err}")),
            "demo",
            extension_id(),
            "cmd.run",
            Arc::clone(&runtime),
        );
        assert_eq!(
            command
                .execute(json!({}))
                .unwrap_or_else(|err| panic!("command: {err}"))["message"],
            "done"
        );
        let failing = ExtensionCommandAdapter::new(
            ContributionId::new("bad_command").unwrap_or_else(|err| panic!("valid id: {err}")),
            "bad",
            extension_id(),
            "cmd.err",
            runtime,
        );
        assert!(matches!(
            failing.execute(json!({})),
            Err(RuntimeError::Handler(_))
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
