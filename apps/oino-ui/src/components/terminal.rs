//! Interactive terminal (design.md A1.5): the bottom frame's **Terminal** tab. Built like Zed's:
//! `alacritty_terminal` emulates (grid, scrollback, selection, modes) over the platform PTY
//! (`alacritty_terminal::tty`: `openpty` on Unix, `ConPTY` on Windows), and the grid is painted
//! with GPUI text runs, one run per same-style cell span, positioned on a fixed cell grid.
//!
//! - Keys: printable text, Enter/Tab/Backspace/Esc, arrows (application-cursor aware),
//!   Home/End/PageUp/PageDown/Delete/Insert/F1–F12, Ctrl+letter, Alt as ESC prefix.
//! - Copy / paste: ⌘C/⌘V on macOS, Ctrl+Shift+C/V elsewhere (bracketed paste when the app
//!   asked for it). Selecting with the mouse copies on ⌘C.
//! - Links: ⌘/Ctrl+click on a URL emits [`TerminalEvent::OpenUrl`] (the pane opens it in the
//!   Browser tab).
//! - Scrollback: 10 000 lines; wheel and Shift+PageUp/PageDown scroll (alternate-screen apps
//!   get arrow keys instead).
//! - Theme: ANSI colors, foreground, cursor (`cream` in dark) and selection from the tokens.

use std::borrow::Cow;
use std::cell::Cell as StdCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};
use futures::StreamExt as _;
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font, FontStyle,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, ScrollEvent,
    SharedString, Styled as _, Task, TextAlign, TextRun, Window, canvas, div, fill, font, point,
    px, size as gsize,
};

use crate::design::space;
use crate::design::theme::{OinoTheme, color};

/// Lines kept above the screen.
pub const SCROLLBACK: usize = 10_000;

/// Emitted to the pane.
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    /// The shell set a title.
    Title(String),
    /// The shell exited.
    Exited,
    /// ⌘/Ctrl+click on a link.
    OpenUrl(String),
}

/// Forwards emulator events to the view (any thread → GPUI task).
#[derive(Clone)]
struct Listener(futures::channel::mpsc::UnboundedSender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

/// One styled span of a line.
#[derive(Clone, Debug, PartialEq)]
struct Span {
    col: usize,
    text: String,
    cells: usize,
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// What one frame paints (taken under the terminal lock, painted without it).
#[derive(Default)]
struct Snapshot {
    lines: Vec<Vec<Span>>,
    cursor: Option<(usize, usize)>,
}

/// One terminal (one shell process).
pub struct TerminalView {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    focus: FocusHandle,
    title: String,
    exited: bool,
    grid: (usize, usize),
    cell: (Pixels, Pixels),
    origin: Rc<StdCell<gpui_kit::Point<Pixels>>>,
    selecting: bool,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// `0xRRGGBBAA` of xterm-256 color `i` (16–255: 6×6×6 cube, then 24 grays).
#[must_use]
pub fn xterm_256(i: u8) -> u32 {
    let to = |r: u8, g: u8, b: u8| u32::from_be_bytes([r, g, b, 0xFF]);
    match i {
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            to(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            to(v, v, v)
        }
        _ => 0,
    }
}

/// Bytes a keystroke sends to the PTY (`None` = not a terminal key).
#[must_use]
pub fn key_bytes(
    key: &str,
    key_char: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let csi = |s: &str| format!("\x1b[{s}").into_bytes();
    let ss3 = |s: &str| format!("\x1bO{s}").into_bytes();
    let arrow = |c: &str| if app_cursor { ss3(c) } else { csi(c) };
    let mut out = match key {
        "enter" => vec![b'\r'],
        "tab" if shift => csi("Z"),
        "tab" => vec![b'\t'],
        "backspace" => vec![if ctrl { 0x08 } else { 0x7f }],
        "escape" => vec![0x1b],
        "up" => arrow("A"),
        "down" => arrow("B"),
        "right" => arrow("C"),
        "left" => arrow("D"),
        "home" => arrow("H"),
        "end" => arrow("F"),
        "pageup" => csi("5~"),
        "pagedown" => csi("6~"),
        "delete" => csi("3~"),
        "insert" => csi("2~"),
        "f1" => ss3("P"),
        "f2" => ss3("Q"),
        "f3" => ss3("R"),
        "f4" => ss3("S"),
        "f5" => csi("15~"),
        "f6" => csi("17~"),
        "f7" => csi("18~"),
        "f8" => csi("19~"),
        "f9" => csi("20~"),
        "f10" => csi("21~"),
        "f11" => csi("23~"),
        "f12" => csi("24~"),
        "space" if ctrl => vec![0],
        k if ctrl && k.chars().count() == 1 => {
            let c = k.chars().next()?.to_ascii_lowercase();
            match c {
                'a'..='z' => vec![(c as u8) & 0x1f],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                '^' | '6' => vec![0x1e],
                '_' | '-' => vec![0x1f],
                '@' | '2' => vec![0],
                _ => return None,
            }
        }
        _ => key_char?.as_bytes().to_vec(),
    };
    if alt && !out.is_empty() && out[0] != 0x1b {
        out.insert(0, 0x1b);
    }
    Some(out)
}

/// The URL around char column `col` of `line`, if any.
#[must_use]
pub fn url_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if col >= chars.len() || chars[col].is_whitespace() {
        return None;
    }
    let stop = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`');
    let mut start = col;
    while start > 0 && !stop(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && !stop(chars[end]) {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    let word = word.trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);
    let word = word.trim_start_matches(['(', '[', '{']);
    (word.starts_with("http://") || word.starts_with("https://") || word.starts_with("file://"))
        .then(|| word.to_owned())
}

/// A started shell, before it gets its view ([`TerminalView::new`]).
pub struct Shell {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    rx: futures::channel::mpsc::UnboundedReceiver<TermEvent>,
    grid: (usize, usize),
}

/// Typed `OINO_DESKTOP_TERM_INPUT` already (once per process).
static TYPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static NEXT_PTY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Shell {
    /// Spawns the user's shell in `cwd` on a new PTY.
    ///
    /// # Errors
    /// The PTY could not be opened or its I/O thread not started.
    pub fn spawn(cwd: &Path) -> std::io::Result<Self> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let listener = Listener(tx);
        let (cols, lines) = (80usize, 24usize);
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        let term = Term::new(config, &TermSize::new(cols, lines), listener.clone());
        let term = Arc::new(FairMutex::new(term));
        let mut env = HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        env.insert("TERM_PROGRAM".to_owned(), "oino".to_owned());
        let options = tty::Options {
            shell: None,
            working_directory: cwd.is_dir().then(|| cwd.to_path_buf()),
            drain_on_exit: true,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let id = NEXT_PTY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pty = tty::new(&options, window_size, id)?;
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _io = event_loop.spawn();
        Ok(Self {
            term,
            sender,
            rx,
            grid: (cols, lines),
        })
    }
}

impl TerminalView {
    /// The view of a started shell.
    pub fn new(shell: Shell, cx: &mut Context<Self>) -> Self {
        let Shell {
            term,
            sender,
            mut rx,
            grid,
        } = shell;
        let events = cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Coalesce a burst of wakeups into one redraw.
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this.update(cx, |view, cx| {
                    for event in batch {
                        view.on_term_event(event, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        // Screenshot hook: type a command into the first terminal of the process.
        if let Ok(input) = std::env::var("OINO_DESKTOP_TERM_INPUT")
            && !TYPED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let _ = sender.send(Msg::Input(format!("{input}\r").into_bytes().into()));
        }
        Self {
            term,
            sender,
            focus: cx.focus_handle(),
            title: String::new(),
            exited: false,
            grid,
            cell: (px(space::S2), px(space::S4)),
            origin: Rc::new(StdCell::new(gpui_kit::Point::default())),
            selecting: false,
            _events: events,
        }
    }

    /// The shell title (or empty).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the shell exited.
    #[must_use]
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// Moves keyboard focus into the terminal.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    fn on_term_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Title(title) => {
                self.title.clone_from(&title);
                cx.emit(TerminalEvent::Title(title));
            }
            TermEvent::ResetTitle => self.title.clear(),
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            TermEvent::ClipboardLoad(_, format) => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let size = self.window_size();
                self.write(format(size).into_bytes());
            }
            TermEvent::ColorRequest(index, format) => {
                let t = OinoTheme::get(cx);
                let c = if index < 16 {
                    t.p.ansi[index]
                } else if index == 256 + 1 {
                    t.p.frame
                } else {
                    t.p.fg
                };
                let [r, g, b, _] = c.to_be_bytes();
                self.write(format(alacritty_terminal::vte::ansi::Rgb { r, g, b }).into_bytes());
            }
            TermEvent::Exit | TermEvent::ChildExit(_) => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited);
                }
            }
            TermEvent::Wakeup
            | TermEvent::Bell
            | TermEvent::MouseCursorDirty
            | TermEvent::CursorBlinkingChange => {}
        }
    }

    fn window_size(&self) -> WindowSize {
        let (cols, lines) = self.grid;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "cell metrics are small positive pixels"
        )]
        WindowSize {
            num_lines: u16::try_from(lines).unwrap_or(u16::MAX),
            num_cols: u16::try_from(cols).unwrap_or(u16::MAX),
            cell_width: f32::from(self.cell.0).round() as u16,
            cell_height: f32::from(self.cell.1).round() as u16,
        }
    }

    /// Resizes the grid to fit `bounds` (called from paint; cheap when unchanged).
    fn fit(&mut self, bounds: Bounds<Pixels>) {
        let (cw, lh) = (f32::from(self.cell.0), f32::from(self.cell.1));
        if cw <= 0.0 || lh <= 0.0 {
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "floor of a positive pixel ratio"
        )]
        let (cols, lines) = (
            ((f32::from(bounds.size.width) / cw).floor() as usize).max(2),
            ((f32::from(bounds.size.height) / lh).floor() as usize).max(1),
        );
        if (cols, lines) == self.grid {
            return;
        }
        self.grid = (cols, lines);
        self.term.lock().resize(TermSize::new(cols, lines));
        let _ = self.sender.send(Msg::Resize(self.window_size()));
    }

    fn cell_at(&self, position: gpui_kit::Point<Pixels>) -> (TermPoint, Side) {
        let origin = self.origin.get();
        let x = f32::from(position.x - origin.x).max(0.0);
        let y = f32::from(position.y - origin.y).max(0.0);
        let (cw, lh) = (f32::from(self.cell.0), f32::from(self.cell.1));
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_possible_wrap,
            reason = "grid coordinates are small and clamped"
        )]
        let (col, row) = (
            ((x / cw) as usize).min(self.grid.0.saturating_sub(1)),
            ((y / lh) as i32).min(i32::try_from(self.grid.1).unwrap_or(1) - 1),
        );
        let side = if (x / cw).fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let offset = i32::try_from(self.term.lock().grid().display_offset()).unwrap_or(0);
        (TermPoint::new(Line(row - offset), Column(col)), side)
    }

    fn line_text(&self, line: Line) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        if line.0 < -i32::try_from(grid.history_size()).unwrap_or(0)
            || line.0 >= i32::try_from(grid.screen_lines()).unwrap_or(0)
        {
            return String::new();
        }
        let row = &grid[line];
        (0..grid.columns()).map(|c| row[Column(c)].c).collect()
    }

    fn copy(&self, cx: &mut App) -> bool {
        let text = self.term.lock().selection_to_string();
        match text.filter(|s| !s.is_empty()) {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            None => false,
        }
    }

    fn paste(&self, cx: &mut App) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let mac = cfg!(target_os = "macos");
        let copy_combo = if mac {
            m.platform && k.key == "c"
        } else {
            m.control && m.shift && k.key == "c"
        };
        let paste_combo = if mac {
            m.platform && k.key == "v"
        } else {
            m.control && m.shift && k.key == "v"
        };
        if copy_combo {
            self.copy(cx);
            cx.stop_propagation();
            return;
        }
        if paste_combo {
            self.paste(cx);
            cx.stop_propagation();
            return;
        }
        if m.shift && matches!(k.key.as_str(), "pageup" | "pagedown") {
            self.term.lock().scroll_display(if k.key == "pageup" {
                Scroll::PageUp
            } else {
                Scroll::PageDown
            });
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // App shortcuts (⌘… on macOS) stay with the app.
        if m.platform {
            return;
        }
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_bytes(
            &k.key,
            k.key_char.as_deref(),
            m.control,
            m.alt,
            m.shift,
            app_cursor,
        ) {
            {
                let mut term = self.term.lock();
                term.scroll_display(Scroll::Bottom);
                term.selection = None;
            }
            self.write(bytes);
            cx.stop_propagation();
        }
    }

    fn snapshot(&self, t: &OinoTheme) -> Snapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let lines = term.screen_lines();
        let offset = content.display_offset;
        let colors = content.colors;
        let resolve = |c: AnsiColor, fg: bool| -> Option<Hsla> {
            let rgba = match c {
                AnsiColor::Spec(s) => u32::from_be_bytes([s.r, s.g, s.b, 0xFF]),
                AnsiColor::Indexed(i) => match colors[usize::from(i)] {
                    Some(s) => u32::from_be_bytes([s.r, s.g, s.b, 0xFF]),
                    None if i < 16 => t.p.ansi[usize::from(i)],
                    None => xterm_256(i),
                },
                AnsiColor::Named(n) => {
                    let idx = n as usize;
                    if let Some(s) = colors[n] {
                        u32::from_be_bytes([s.r, s.g, s.b, 0xFF])
                    } else if idx < 16 {
                        t.p.ansi[idx]
                    } else {
                        match n {
                            NamedColor::Background => return None,
                            NamedColor::Cursor => t.p.cream,
                            NamedColor::DimForeground => t.p.fg_subtle,
                            NamedColor::DimBlack
                            | NamedColor::DimRed
                            | NamedColor::DimGreen
                            | NamedColor::DimYellow
                            | NamedColor::DimBlue
                            | NamedColor::DimMagenta
                            | NamedColor::DimCyan
                            | NamedColor::DimWhite => {
                                (t.p.ansi[idx - NamedColor::DimBlack as usize] & 0xFFFF_FF00) | 0xA0
                            }
                            _ => t.p.fg,
                        }
                    }
                }
            };
            if !fg && rgba == t.p.frame {
                return None;
            }
            Some(color(rgba))
        };
        let selection = content.selection;
        let mut out: Vec<Vec<Span>> = vec![Vec::new(); lines];
        for cell in content.display_iter {
            let Ok(row) = usize::try_from(cell.point.line.0 + i32::try_from(offset).unwrap_or(0))
            else {
                continue;
            };
            if row >= lines
                || cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let col = cell.point.column.0;
            let mut fg = resolve(cell.fg, true).unwrap_or_else(|| color(t.p.fg));
            let mut bg = resolve(cell.bg, false);
            if cell.flags.contains(Flags::INVERSE) {
                let old = fg;
                fg = bg.unwrap_or_else(|| color(t.p.frame));
                bg = Some(old);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.66);
            }
            if selection.is_some_and(|s| s.contains(cell.point)) {
                bg = Some(color(t.p.selection));
            }
            let ch = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            let cells = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let underline = cell.flags.intersects(Flags::ALL_UNDERLINES);
            let spans = &mut out[row];
            match spans.last_mut() {
                Some(last)
                    if last.col + last.cells == col
                        && last.fg == fg
                        && last.bg == bg
                        && last.bold == bold
                        && last.italic == italic
                        && last.underline == underline
                        && cells == 1
                        && !last.text.ends_with(|c: char| !c.is_ascii()) =>
                {
                    last.text.push(ch);
                    last.cells += 1;
                }
                _ => spans.push(Span {
                    col,
                    text: ch.to_string(),
                    cells,
                    fg,
                    bg,
                    bold,
                    italic,
                    underline,
                }),
            }
        }
        // Trailing blank default spans paint nothing.
        for spans in &mut out {
            spans.retain(|s| s.bg.is_some() || s.underline || !s.text.trim().is_empty());
        }
        let cursor = (content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0)
            .then(|| {
                let p = content.cursor.point;
                usize::try_from(p.line.0).ok().map(|row| (p.column.0, row))
            })
            .flatten();
        Snapshot { lines: out, cursor }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let (font_size, line_height) = t.code_text();
        let base: Font = font(t.mono_font());
        let font_id = window.text_system().resolve_font(&base);
        let cw = window
            .text_system()
            .advance(font_id, font_size, 'm')
            .map_or(font_size * 0.6, |s| s.width);
        self.cell = (cw, line_height);
        let snapshot = self.snapshot(&t);
        let focused = self.focus.is_focused(window);
        let cursor_color = color(if t.dark { t.p.cream } else { t.p.fg });
        let view = cx.entity().downgrade();
        let origin = Rc::clone(&self.origin);
        let exited = self.exited;
        div()
            .id("terminal")
            .size_full()
            .track_focus(&self.focus)
            .key_context("Terminal")
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    let (p, side) = this.cell_at(e.position);
                    if crate::components::secondary(&e.modifiers) {
                        if let Some(url) = url_at(&this.line_text(p.line), p.column.0) {
                            cx.emit(TerminalEvent::OpenUrl(url));
                        }
                        return;
                    }
                    let ty = match e.click_count {
                        2 => SelectionType::Semantic,
                        3 => SelectionType::Lines,
                        _ => SelectionType::Simple,
                    };
                    this.term.lock().selection = Some(Selection::new(ty, p, side));
                    this.selecting = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if this.selecting && e.pressed_button == Some(MouseButton::Left) {
                    let (p, side) = this.cell_at(e.position);
                    if let Some(sel) = this.term.lock().selection.as_mut() {
                        sel.update(p, side);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.selecting = false;
                    let empty = this
                        .term
                        .lock()
                        .selection
                        .as_ref()
                        .is_none_or(Selection::is_empty);
                    if empty {
                        this.term.lock().selection = None;
                    }
                    cx.notify();
                }),
            )
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                let dy = f32::from(e.delta.pixel_delta(this.cell.1).y);
                #[allow(clippy::cast_possible_truncation, reason = "a few lines per event")]
                let lines = (dy / f32::from(this.cell.1)).round() as i32;
                if lines == 0 {
                    return;
                }
                let alt = this.term.lock().mode().contains(TermMode::ALT_SCREEN);
                if alt {
                    let key: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                    for _ in 0..lines.unsigned_abs() {
                        this.write(key.to_vec());
                    }
                } else {
                    this.term.lock().scroll_display(Scroll::Delta(lines));
                }
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        origin.set(bounds.origin);
                        let _ = view.update(cx, |v, _| v.fit(bounds));
                    },
                    move |bounds, (), window, cx| {
                        let o = bounds.origin;
                        for (row, spans) in snapshot.lines.iter().enumerate() {
                            #[allow(clippy::cast_precision_loss, reason = "screen rows")]
                            let y = o.y + line_height * row as f32;
                            for s in spans {
                                #[allow(clippy::cast_precision_loss, reason = "screen columns")]
                                let x = o.x + cw * s.col as f32;
                                if let Some(bg) = s.bg {
                                    #[allow(clippy::cast_precision_loss, reason = "span cells")]
                                    let w = cw * s.cells as f32;
                                    window.paint_quad(fill(
                                        Bounds::new(point(x, y), gsize(w, line_height)),
                                        bg,
                                    ));
                                }
                                if s.text.trim().is_empty() && !s.underline {
                                    continue;
                                }
                                let mut f = base.clone();
                                if s.bold {
                                    f.weight = FontWeight::BOLD;
                                }
                                if s.italic {
                                    f.style = FontStyle::Italic;
                                }
                                let run = TextRun {
                                    len: s.text.len(),
                                    font: f,
                                    color: s.fg,
                                    background_color: None,
                                    underline: s.underline.then(|| gpui_kit::UnderlineStyle {
                                        thickness: px(space::HALF / 2.0),
                                        color: Some(s.fg),
                                        wavy: false,
                                    }),
                                    strikethrough: None,
                                };
                                let shaped = window.text_system().shape_line(
                                    SharedString::from(s.text.clone()),
                                    font_size,
                                    &[run],
                                    None,
                                );
                                let _ = shaped.paint(
                                    point(x, y),
                                    line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                        if let Some((col, row)) = snapshot.cursor.filter(|_| !exited) {
                            #[allow(clippy::cast_precision_loss, reason = "screen position")]
                            let at = point(o.x + cw * col as f32, o.y + line_height * row as f32);
                            let quad = if focused {
                                fill(
                                    Bounds::new(at, gsize(cw, line_height)),
                                    cursor_color.opacity(0.7),
                                )
                            } else {
                                gpui_kit::outline(
                                    Bounds::new(at, gsize(cw, line_height)),
                                    cursor_color,
                                    gpui_kit::BorderStyle::Solid,
                                )
                            };
                            window.paint_quad(quad);
                        }
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_pty_bytes() {
        assert_eq!(
            key_bytes("enter", None, false, false, false, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            key_bytes("c", Some("c"), true, false, false, false),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes("up", None, false, false, false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes("up", None, false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes("x", Some("x"), false, true, false, false),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            key_bytes("é", Some("é"), false, false, false, false),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(key_bytes("shift", None, false, false, true, false), None);
    }

    #[test]
    fn colors_and_links() {
        assert_eq!(xterm_256(16), 0x0000_00FF);
        assert_eq!(xterm_256(231), 0xFFFF_FFFF);
        assert_eq!(xterm_256(232), 0x0808_08FF);
        let line = "  ➜  Local:   http://localhost:5173/ (ready)";
        let col = line.chars().position(|c| c == 'h').unwrap_or(0) + 3;
        assert_eq!(url_at(line, col).as_deref(), Some("http://localhost:5173/"));
        assert_eq!(url_at(line, 0), None);
    }
}
