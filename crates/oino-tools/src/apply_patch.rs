//! Codex file patch syntax, offered only to GPT/Codex/o-series models.

use futures::{FutureExt, future::BoxFuture};
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use tokio_util::sync::CancellationToken;

use crate::edit::storage::{TextFile, atomic_write, check_observed, observe};
use crate::{ToolConfig, aborted, args};

/// Whether a model id is eligible for `apply_patch` (provider prefixes are allowed).
#[must_use]
pub fn model_supports_apply_patch(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let leaf = model.rsplit('/').next().unwrap_or(&model);
    model.contains("gpt")
        || model.contains("codex")
        || leaf
            .strip_prefix('o')
            .is_some_and(|rest| rest.starts_with(|ch: char| ch.is_ascii_digit()))
}

/// Patch tool. The registry gates declarations; execution rechecks the live model.
#[derive(Debug)]
pub struct ApplyPatchTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl ApplyPatchTool {
    /// Construct a patch tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: crate::info::apply_patch_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for ApplyPatchTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }
    #[allow(clippy::too_many_lines, reason = "single tool execution flow")]
    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        async move {
            let model = config
                .context
                .session_env()
                .and_then(|session| session.model)
                .unwrap_or_default();
            if !model_supports_apply_patch(&model) {
                return Err(ToolError::new("apply_patch is unavailable for this model"));
            }
            let operations = parse(args::required_str(&call.args, "input")?)?;
            let mut summary = Vec::new();
            for operation in operations {
                if cancel.is_cancelled() {
                    return Err(aborted());
                }
                let path =
                    crate::path_utils::resolve_to_cwd(&operation.path, &config.effective_cwd())
                        .map_err(|error| ToolError::new(error.to_string()))?;
                let env = config.env.as_ref();
                config
                    .mutation_queue
                    .run(env, &path, async {
                        let previous = env.read_file(&path).await;
                        match operation.kind {
                            Kind::Add(text) => {
                                if previous.is_ok() || env.stat(&path).await.is_ok() {
                                    return Err(ToolError::new(format!(
                                        "File already exists: {path}"
                                    )));
                                }
                                let text_file = TextFile::decode(text.as_bytes())?;
                                let bytes = text_file.encode(&text, true);
                                let guarantee = atomic_write(&config, &path, &bytes, None).await?;
                                observe(&config, &path, &bytes)?;
                                summary.push(format!("A {path}{}", guarantee.notice()));
                            }
                            Kind::Delete => {
                                if !env.is_local() {
                                    return Err(ToolError::new(
                                        "Patch deletion requires a local environment",
                                    ));
                                }
                                let bytes =
                                    previous.map_err(|error| ToolError::new(error.to_string()))?;
                                check_observed(&config, &path, &bytes)?;
                                tokio::fs::remove_file(&path)
                                    .await
                                    .map_err(|error| ToolError::new(error.to_string()))?;
                                summary.push(format!("D {path}"));
                            }
                            Kind::Update { hunks, destination } => {
                                let bytes =
                                    previous.map_err(|error| ToolError::new(error.to_string()))?;
                                check_observed(&config, &path, &bytes)?;
                                let file = TextFile::decode(&bytes)?;
                                let updated = update(&file.text.replace("\r\n", "\n"), &hunks)?;
                                let encoded = file.encode(&updated, false);
                                let target = destination
                                    .map(|target| {
                                        crate::path_utils::resolve_to_cwd(
                                            &target,
                                            &config.effective_cwd(),
                                        )
                                    })
                                    .transpose()
                                    .map_err(|error| ToolError::new(error.to_string()))?
                                    .unwrap_or_else(|| path.clone());
                                if target != path && env.stat(&target).await.is_ok() {
                                    return Err(ToolError::new(
                                        "Patch move destination already exists",
                                    ));
                                }
                                let guarantee = atomic_write(
                                    &config,
                                    &target,
                                    &encoded,
                                    if target == path { Some(&bytes) } else { None },
                                )
                                .await?;
                                if target != path {
                                    if env
                                        .read_file(&path)
                                        .await
                                        .map_err(|error| ToolError::new(error.to_string()))?
                                        != bytes
                                    {
                                        return Err(ToolError::new(
                                            "Patch source changed while moving",
                                        ));
                                    }
                                    tokio::fs::remove_file(&path)
                                        .await
                                        .map_err(|error| ToolError::new(error.to_string()))?;
                                }
                                observe(&config, &target, &encoded)?;
                                summary.push(format!("M {target}{}", guarantee.notice()));
                            }
                        }
                        Ok(())
                    })
                    .await?;
            }
            Ok(AgentToolResult::text(format!(
                "Success. Updated the following files:\n{}",
                summary.join("\n")
            )))
        }
        .boxed()
    }
}

struct Operation {
    path: String,
    kind: Kind,
}
enum Kind {
    Add(String),
    Delete,
    Update {
        hunks: Vec<Hunk>,
        destination: Option<String>,
    },
}
struct Hunk {
    anchor: Option<String>,
    lines: Vec<String>,
    eof: bool,
}

fn parse(input: &str) -> Result<Vec<Operation>, ToolError> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err(ToolError::new(
            "Patch must start with *** Begin Patch and end with *** End Patch",
        ));
    }
    let mut result = Vec::new();
    let mut index = 1;
    while index + 1 < lines.len() {
        let header = lines[index];
        index += 1;
        let (path, kind) = if let Some(path) = header.strip_prefix("*** Add File: ") {
            let mut content = String::new();
            while index + 1 < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index]
                    .strip_prefix('+')
                    .ok_or_else(|| ToolError::new("Added lines must start with +"))?;
                content.push_str(line);
                content.push('\n');
                index += 1;
            }
            (path, Kind::Add(content))
        } else if let Some(path) = header.strip_prefix("*** Delete File: ") {
            (path, Kind::Delete)
        } else if let Some(path) = header.strip_prefix("*** Update File: ") {
            let destination = lines
                .get(index)
                .and_then(|line| line.strip_prefix("*** Move to: "))
                .map(str::to_owned);
            if destination.is_some() {
                index += 1;
            }
            let mut hunks: Vec<Hunk> = Vec::new();
            while index + 1 < lines.len() {
                let line = lines[index];
                if line == "*** End of File" {
                    hunks
                        .last_mut()
                        .ok_or_else(|| ToolError::new("End of File requires a hunk"))?
                        .eof = true;
                } else if line == "@@" || line.starts_with("@@ ") {
                    hunks.push(Hunk {
                        anchor: line.strip_prefix("@@ ").map(str::to_owned),
                        lines: Vec::new(),
                        eof: false,
                    });
                } else if line.starts_with("*** ") {
                    break;
                } else if line.starts_with([' ', '+', '-']) {
                    if hunks.is_empty() {
                        hunks.push(Hunk {
                            anchor: None,
                            lines: Vec::new(),
                            eof: false,
                        });
                    }
                    hunks
                        .last_mut()
                        .ok_or_else(|| ToolError::new("Missing hunk"))?
                        .lines
                        .push(line.to_owned());
                } else {
                    return Err(ToolError::new("Invalid patch hunk line"));
                }
                index += 1;
            }
            if hunks.is_empty() || hunks.iter().any(|hunk| hunk.lines.is_empty()) {
                return Err(ToolError::new("Update requires nonempty hunks"));
            }
            (path, Kind::Update { hunks, destination })
        } else {
            return Err(ToolError::new(format!("Invalid patch header: {header}")));
        };
        if path.is_empty() {
            return Err(ToolError::new("Patch path must not be empty"));
        }
        result.push(Operation {
            path: path.to_owned(),
            kind,
        });
    }
    if result.is_empty() {
        return Err(ToolError::new("Patch contains no operations"));
    }
    Ok(result)
}

fn update(text: &str, hunks: &[Hunk]) -> Result<String, ToolError> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut cursor = 0;
    for hunk in hunks {
        if let Some(anchor) = &hunk.anchor {
            cursor = lines
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, line)| *line == anchor)
                .map(|(index, _)| index + 1)
                .ok_or_else(|| ToolError::new("Patch anchor not found"))?;
        }
        let old: Vec<String> = hunk
            .lines
            .iter()
            .filter(|line| !line.starts_with('+'))
            .map(|line| line[1..].to_owned())
            .collect();
        let new: Vec<String> = hunk
            .lines
            .iter()
            .filter(|line| !line.starts_with('-'))
            .map(|line| line[1..].to_owned())
            .collect();
        let start = if old.is_empty() {
            if hunk.eof { lines.len() } else { cursor }
        } else {
            (cursor..=lines.len().saturating_sub(old.len()))
                .find(|&start| {
                    lines.get(start..start + old.len()) == Some(old.as_slice())
                        && (!hunk.eof || start + old.len() == lines.len())
                })
                .ok_or_else(|| ToolError::new("Patch context not found"))?
        };
        cursor = start + new.len();
        lines.splice(start..start + old.len(), new);
    }
    let mut result = lines.join("\n");
    if text.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    Ok(result)
}
