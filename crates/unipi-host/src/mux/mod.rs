pub mod herdr;
pub mod shell;
pub mod tmux;

use unipi_proto::{
    AppErrorCode, AppErrorDto, AttachTarget, ListData, MuxAction, MuxKind, PreviewData,
    SessionName, ShortcutsData, SizePolicy, TreeData,
};

/// Runs `herdr session list --json` and parses it into `ListData`.
/// tmux lists come from the `tree` op, not `list`.
pub async fn list(mux: MuxKind) -> Result<ListData, AppErrorDto> {
    match mux {
        MuxKind::Herdr => herdr::list().await,
        MuxKind::Tmux => Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "tmux sessions come from the tree op",
            false,
        )),
    }
}

/// Both multiplexer trees in one reply (DESIGN.md §5.6), plus the
/// protected session names so the app can dim them.
pub async fn tree(protected: &[String]) -> Result<TreeData, AppErrorDto> {
    let (herdr, tmux) = tokio::join!(herdr::tree(), tmux::tree());
    Ok(TreeData {
        protected: protected
            .iter()
            .map(|name| {
                SessionName::try_from(name.clone())
                    .unwrap_or_else(|_| SessionName::try_from(String::new()).unwrap())
            })
            .collect(),
        herdr: herdr?,
        tmux: tmux?,
    })
}

/// Applies a multiplexer mutation/navigation (DESIGN.md §3.3).
pub async fn action(action: &MuxAction) -> Result<serde_json::Value, AppErrorDto> {
    match action {
        MuxAction::WorkspaceCreate { .. }
        | MuxAction::WorkspaceRename { .. }
        | MuxAction::WorkspaceClose { .. }
        | MuxAction::TabNext { .. }
        | MuxAction::TabPrev { .. }
        | MuxAction::HerdrPaneNext { .. }
        | MuxAction::HerdrPanePrev { .. } => herdr::action(action).await,
        MuxAction::TmuxSessionCreate { .. }
        | MuxAction::TmuxSessionRename { .. }
        | MuxAction::TmuxSessionKill { .. }
        | MuxAction::WindowNext { .. }
        | MuxAction::WindowPrev { .. }
        | MuxAction::TmuxPaneNext { .. }
        | MuxAction::TmuxPanePrev { .. } => tmux::action(action).await,
    }
}

/// The shortcuts panel data (DESIGN.md §3.3).
pub async fn shortcuts() -> Result<ShortcutsData, AppErrorDto> {
    herdr::shortcuts().await
}

/// ANSI preview text for the focused pane of `target`.
pub async fn preview(target: &AttachTarget, lines: u16) -> Result<PreviewData, AppErrorDto> {
    match target {
        AttachTarget::Herdr { .. } => herdr::preview(target, lines).await,
        AttachTarget::Tmux { .. } => tmux::preview(target, lines).await,
    }
}

/// Prepares an attach: applies focus for herdr, creates the grouped
/// tmux session for tmux, and returns the policy actually applied
/// plus the PTY command to spawn.
pub async fn attach_setup(
    target: &AttachTarget,
    cols: u16,
    rows: u16,
    size_policy: SizePolicy,
) -> Result<(SizePolicy, portable_pty::CommandBuilder, Option<String>), AppErrorDto> {
    match target {
        AttachTarget::Herdr {
            session,
            workspace_id,
            tab_id,
            pane_id,
        } => {
            herdr::focus(
                session.as_str(),
                workspace_id.as_ref(),
                tab_id.as_ref(),
                pane_id.as_ref(),
            )
            .await?;
            Ok((
                SizePolicy::FitPhone,
                herdr::client_command(session.as_str(), cols, rows),
                None,
            ))
        }
        AttachTarget::Tmux {
            session,
            window_id,
            pane_id,
        } => {
            let grouped =
                tmux::create_grouped(session.as_str(), window_id.as_ref(), pane_id.as_ref())
                    .await?;
            let effective = if size_policy == SizePolicy::KeepPc {
                SizePolicy::KeepPc
            } else {
                SizePolicy::FitPhone
            };
            Ok((
                effective,
                tmux::attach_command(&grouped, effective == SizePolicy::KeepPc),
                Some(grouped),
            ))
        }
    }
}

/// Streams the already-configured PTY command: frames 1/2 carry
/// bytes, JSON carries resize/exit, and closing the stream kills the
/// client only.
pub async fn attach_stream<R: tokio::io::AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(
    read: &mut R,
    write: &mut W,
    command: portable_pty::CommandBuilder,
    cols: u16,
    rows: u16,
) -> Result<(), AppErrorDto> {
    crate::protocol::attach_stream(read, write, command, cols, rows)
        .await
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))
}

/// A plain login shell in `$HOME`.
pub async fn shell_command(
    cols: u16,
    rows: u16,
) -> Result<portable_pty::CommandBuilder, AppErrorDto> {
    shell::command(cols, rows).await
}

pub fn tmux_unsupported() -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_UNSUPPORTED,
        "tmux is not supported in this version of unipi-host (M1 is herdr-only)",
        false,
    )
    .with_hint("tmux support ships in M3")
}
