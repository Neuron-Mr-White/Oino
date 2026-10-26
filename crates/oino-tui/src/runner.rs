//! Terminal lifecycle and the event loop: crossterm input, backend events, ticks and the
//! effects the [`App`] queues (commands, dialogs answers, clipboard, editor, suspend).

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use oino_client::SessionBackend;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, EditPurpose, Effect, Tag, TuiOptions};
use crate::host::TuiHost;
use crate::{TuiError, input, render};

type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

/// Restores the terminal on drop (also on panic via the hook installed in [`run`]).
struct TerminalGuard {
    enhanced: bool,
}

fn setup(enhanced: bool) -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    if enhanced {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
    }
    Ok(())
}

fn restore(enhanced: bool) {
    let mut out = std::io::stdout();
    if enhanced {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableBracketedPaste, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    let _ = execute!(out, crossterm::cursor::Show);
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore(self.enhanced);
    }
}

fn osc(sequence: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(sequence.as_bytes());
    let _ = out.flush();
}

/// Runs the interactive client until the user quits.
///
/// # Errors
///
/// Terminal setup or drawing failures.
pub async fn run(
    backend: Arc<dyn SessionBackend>,
    host: Arc<dyn TuiHost>,
    options: TuiOptions,
) -> Result<(), TuiError> {
    let enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore(enhanced);
        previous(info);
    }));
    setup(enhanced)?;
    let guard = TerminalGuard { enhanced };
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let result = event_loop(
        &mut terminal,
        backend.as_ref(),
        &backend,
        host,
        options,
        enhanced,
    )
    .await;
    drop(guard);
    backend.shutdown().await;
    result
}

#[allow(
    clippy::too_many_lines,
    reason = "the select loop and its effect dispatch read best in one place"
)]
async fn event_loop(
    terminal: &mut Term,
    backend: &dyn SessionBackend,
    shared: &Arc<dyn SessionBackend>,
    host: Arc<dyn TuiHost>,
    options: TuiOptions,
    enhanced: bool,
) -> Result<(), TuiError> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (queue_tx, mut queue_rx) =
        mpsc::unbounded_channel::<(Tag, oino_client::protocol::RpcCommand, Option<String>)>();
    // Ordered command queue (replies keep command order).
    {
        let backend = Arc::clone(shared);
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some((tag, command, id)) = queue_rx.recv().await {
                let result = backend.send_with_id(id, command).await;
                if tx.send(AppEvent::Reply(tag, result)).is_err() {
                    break;
                }
            }
        });
    }
    let mut app = App::new(Arc::clone(&host), options);
    let mut events = backend.events();
    let mut input_events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    app.start();
    let mut dirty = true;
    loop {
        // Execute effects.
        for effect in app.take_effects() {
            match effect {
                Effect::Send {
                    tag,
                    command,
                    request_id,
                    concurrent,
                } => {
                    if concurrent {
                        let backend = Arc::clone(shared);
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = backend.send_with_id(request_id, command).await;
                            let _ = tx.send(AppEvent::Reply(tag, result));
                        });
                    } else {
                        let _ = queue_tx.send((tag, command, request_id));
                    }
                }
                Effect::Respond(response) => backend.ui_respond(response),
                Effect::UiAction {
                    panel_id,
                    action_id,
                    value,
                } => {
                    if !host.ui_action(&panel_id, &action_id, value) && action_id != "$close" {
                        let _ = tx.send(AppEvent::Notice(
                            "Panel actions need the `_oino/ui_action` backend command".into(),
                        ));
                    }
                }
                Effect::InvokeShortcut(id) => {
                    if !host.invoke_shortcut(&id) {
                        let _ = tx.send(AppEvent::Notice(format!(
                            "Extension shortcut \"{id}\" needs the `_oino/invoke_shortcut` backend command"
                        )));
                    }
                }
                Effect::ListSessions(scope) => {
                    let host = Arc::clone(&host);
                    let tx = tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = tx.send(AppEvent::Sessions(host.list_sessions(scope)));
                    });
                }
                Effect::DeleteSession(path) => {
                    let result = host.delete_session(&path);
                    let _ = tx.send(AppEvent::SessionDeleted(path, result));
                }
                Effect::ClipboardWrite(text) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
                    osc(&format!("\x1b]52;c;{encoded}\x07"));
                }
                Effect::ClipboardImage(dialog) => {
                    let host = Arc::clone(&host);
                    let tx = tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let image = host.clipboard_image();
                        let text = if image.is_none() && dialog.is_none() {
                            host.clipboard_text()
                        } else {
                            None
                        };
                        let _ = tx.send(AppEvent::Clipboard {
                            dialog,
                            image,
                            text,
                        });
                    });
                }
                Effect::ExternalEditor {
                    text,
                    extension,
                    purpose,
                } => {
                    restore(enhanced);
                    let edited = external_editor(&text, &extension);
                    setup(enhanced)?;
                    terminal.clear()?;
                    match edited {
                        Ok(edited) => app.handle(AppEvent::Edited(purpose, edited)),
                        Err(error) => {
                            if let EditPurpose::External(_) | EditPurpose::Dialog(_) = &purpose {
                                app.handle(AppEvent::Edited(purpose, None));
                            }
                            app.handle(AppEvent::Notice(format!(
                                "External editor failed: {error}"
                            )));
                        }
                    }
                    dirty = true;
                }
                Effect::OsNotify { title, body } => {
                    osc(&format!("\x1b]777;notify;{title};{body}\x07"));
                }
                Effect::OpenUrl(url) => {
                    let opener = if cfg!(target_os = "macos") {
                        "open"
                    } else if cfg!(windows) {
                        "explorer"
                    } else {
                        "xdg-open"
                    };
                    let _ = std::process::Command::new(opener)
                        .arg(&url)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
                Effect::SetTitle(title) => {
                    let _ = execute!(std::io::stdout(), SetTitle(title));
                }
                Effect::Suspend => {
                    restore(enhanced);
                    suspend();
                    setup(enhanced)?;
                    terminal.clear()?;
                    dirty = true;
                }
                Effect::Quit => {}
            }
        }
        if app.should_quit() {
            break;
        }
        if dirty {
            terminal.draw(|frame| render::draw(frame, &mut app))?;
            dirty = false;
        }
        tokio::select! {
            maybe = input_events.next() => match maybe {
                Some(Ok(Event::Key(key))) => {
                    if let Some(id) = input::key_id(&key) {
                        app.handle(AppEvent::Key(id));
                    }
                }
                Some(Ok(Event::Paste(text))) => app.handle(AppEvent::Paste(text)),
                Some(Ok(Event::Resize(_, _))) => app.handle(AppEvent::Resize),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(TuiError::Io(error)),
                None => break,
            },
            maybe = events.next() => if let Some(event) = maybe { app.handle(AppEvent::Client(Box::new(event))) } else {
                app.handle(AppEvent::Notice("Session closed".into()));
                events = Box::pin(futures::stream::pending());
            },
            Some(event) = rx.recv() => app.handle(event),
            _ = tick.tick() => {
                app.handle(AppEvent::Tick);
                // Only redraw on ticks when something animates.
                if !app.animating() {
                    continue;
                }
            }
        }
        dirty = true;
    }
    Ok(())
}

/// pi external editor: `externalEditor` setting, `$VISUAL`, `$EDITOR`, Notepad, `nano`.
fn external_editor(text: &str, extension: &str) -> std::io::Result<Option<String>> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|e| !e.is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|e| !e.is_empty()))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "nano".into()
            }
        });
    let dir = std::env::temp_dir();
    let path = dir.join(format!("oino-edit-{}.{extension}", std::process::id()));
    std::fs::write(&path, text)?;
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("nano");
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()?;
    let result = if status.success() {
        Some(std::fs::read_to_string(&path)?)
    } else {
        None
    };
    let _ = std::fs::remove_file(&path);
    Ok(result)
}

#[cfg(unix)]
fn suspend() {
    let _ = nix::sys::signal::raise(nix::sys::signal::Signal::SIGTSTP);
}

#[cfg(not(unix))]
fn suspend() {}
