//! Core's `!cmd` executor building block (pi `core/bash-executor.ts`
//! `executeBashWithOperations` over the local shell operations).

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::accumulator::{Utf8Decoder, create_output_file};
use super::exec::{ExecEnd, ExecError, ExecRequest, OutputSink, ShellSpec, exec};
use super::shell::{sanitize_binary_output, shell_env, split_incomplete_ansi_suffix, strip_ansi};
use super::temp_dir;
use crate::env::ExecutionEnv;
use crate::js;
use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, truncate_tail};

/// Options of [`run_shell_command`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellRunOptions {
    /// Explicit shell (setting `shellPath`).
    pub shell_path: Option<String>,
    /// Agent directory: temp files in `<agent_dir>/tmp`, `<agent_dir>/bin` on `PATH`.
    pub agent_dir: String,
    /// Base environment; `None` uses this process's environment.
    pub process_env: Option<Vec<(String, String)>>,
}

/// pi `BashResult`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BashOutcome {
    /// Combined, sanitised output (ANSI stripped, binary garbage removed, no `\r`), tail
    /// truncated when needed.
    pub output: String,
    /// Exit code (`None` when cancelled).
    pub exit_code: Option<i32>,
    /// The command was cancelled.
    pub cancelled: bool,
    /// `output` was truncated.
    pub truncated: bool,
    /// Temp file with the full output (written once output exceeded 50KB).
    pub full_output_path: Option<String>,
}

/// A command that could not run (pi rethrows the `exec` error).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ShellRunError(pub String);

struct ExecutorSink<'a> {
    on_chunk: &'a mut (dyn FnMut(&str) + Send),
    decoder: Utf8Decoder,
    pending_ansi: String,
    chunks: VecDeque<String>,
    chunk_units: usize,
    total_bytes: u64,
    temp_dir: String,
    temp: Option<(String, File)>,
}

impl ExecutorSink<'_> {
    fn ensure_temp_file(&mut self) {
        if self.temp.is_some() {
            return;
        }
        if let Ok((path, mut file)) = create_output_file(&self.temp_dir, "oino-bash") {
            for chunk in &self.chunks {
                // pi ignores stream write errors here.
                let _ = file.write_all(chunk.as_bytes());
            }
            self.temp = Some((path, file));
        }
    }

    fn append_text(&mut self, raw: &str) {
        let text = sanitize_binary_output(&strip_ansi(raw)).replace('\r', "");
        if text.is_empty() {
            return;
        }
        if self.total_bytes > DEFAULT_MAX_BYTES {
            self.ensure_temp_file();
        }
        if let Some((_, file)) = &mut self.temp {
            let _ = file.write_all(text.as_bytes());
        }
        let max_units = usize::try_from(DEFAULT_MAX_BYTES * 2).unwrap_or(usize::MAX);
        self.chunk_units += js::utf16_len(&text);
        self.chunks.push_back(text.clone());
        while self.chunk_units > max_units && self.chunks.len() > 1 {
            if let Some(removed) = self.chunks.pop_front() {
                self.chunk_units -= js::utf16_len(&removed);
            }
        }
        (self.on_chunk)(&text);
    }

    fn flush_output(&mut self) {
        let rest = format!(
            "{}{}",
            std::mem::take(&mut self.pending_ansi),
            self.decoder.finish()
        );
        self.append_text(&rest);
    }
}

impl OutputSink for ExecutorSink<'_> {
    fn data(&mut self, chunk: &[u8]) {
        self.total_bytes += chunk.len() as u64;
        let decoded = format!("{}{}", self.pending_ansi, self.decoder.decode(chunk));
        let (complete, pending) = split_incomplete_ansi_suffix(&decoded);
        self.pending_ansi = pending;
        self.append_text(&complete);
    }

    fn next_flush(&self) -> Option<Instant> {
        None
    }

    fn flush(&mut self) {}
}

/// Runs a user `!cmd` like pi `executeBash`: the configured shell, the process environment
/// with `<agent_dir>/bin` on `PATH`, sanitised streaming chunks via `on_chunk`, a rolling
/// buffer and a temp file for long output. Cancelling kills the process tree and returns the
/// output so far with `cancelled: true`.
///
/// # Errors
/// The command could not run (missing working directory, shell not found, spawn failure).
pub async fn run_shell_command(
    env: &dyn ExecutionEnv,
    command: &str,
    cwd: &str,
    options: &ShellRunOptions,
    on_chunk: &mut (dyn FnMut(&str) + Send),
    cancel: &CancellationToken,
) -> Result<BashOutcome, ShellRunError> {
    let mut sink = ExecutorSink {
        on_chunk,
        decoder: Utf8Decoder::default(),
        pending_ansi: String::new(),
        chunks: VecDeque::new(),
        chunk_units: 0,
        total_bytes: 0,
        temp_dir: temp_dir(&options.agent_dir),
        temp: None,
    };
    let request = ExecRequest {
        env,
        shell: ShellSpec::Bash(options.shell_path.as_deref()),
        command: command.to_owned(),
        cwd: cwd.to_owned(),
        vars: shell_env(options.process_env.as_deref(), &options.agent_dir),
        timeout: None,
    };
    let exit_code = match exec(request, &mut sink, cancel, None).await {
        Ok(ExecEnd::Exited(code)) => Some(code),
        Ok(ExecEnd::Detached(..)) | Err(ExecError::Aborted | ExecError::Timeout(_)) => None,
        Err(ExecError::Failed(message)) => {
            if !cancel.is_cancelled() {
                return Err(ShellRunError(message));
            }
            None
        }
    };
    sink.flush_output();
    let full_output: String = sink.chunks.iter().map(String::as_str).collect();
    let truncation = truncate_tail(&full_output, TruncationOptions::default());
    if truncation.truncated {
        sink.ensure_temp_file();
    }
    let cancelled = cancel.is_cancelled();
    let full_output_path = sink.temp.take().map(|(path, mut file)| {
        let _ = file.flush();
        path
    });
    Ok(BashOutcome {
        output: if truncation.truncated {
            truncation.content
        } else {
            full_output
        },
        exit_code: if cancelled { None } else { exit_code },
        cancelled,
        truncated: truncation.truncated,
        full_output_path,
    })
}
