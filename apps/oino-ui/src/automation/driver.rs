//! The UI-thread half of the automation layer (docs/dev/v1/gui-driver.md §2–§3).
//!
//! [`install`] opens the socket and spawns one GPUI task that serves requests strictly in
//! order. Events are injected with [`Window::dispatch_event`], the entry point every platform
//! backend uses, so a scenario exercises the same hit testing, focus transfer, click
//! synthesis, keymap dispatch and IME path as a person at the machine. Requests that would
//! re-enter the view tree (input, quit) run outside the shell's `update_in` — drawing from
//! inside it would double-lease the workspace (stability.md root cause 7).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::{
    App, AppContext as _, AsyncWindowContext, Context, MouseButton, PlatformInput, WeakEntity,
    Window,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::protocol::{Request, error, ok};
use super::registry::{self, Node};
use super::server::{self, Call, Server};
use super::{input, state};
use crate::views::shell::AppShell;

/// Frames a mutating request lets pass before the next one is handled.
const SETTLE_FRAMES: u32 = 2;

/// Longest a settle waits for a frame (a window that is not presenting still answers).
const SETTLE_TIMEOUT: Duration = Duration::from_millis(200);

/// Installs the automation layer when `OINO_DESKTOP_AUTOMATION` names a socket path.
pub fn install(cx: &mut Context<AppShell>, window: &mut Window) {
    let Some(path) = std::env::var_os("OINO_DESKTOP_AUTOMATION").filter(|p| !p.is_empty()) else {
        return;
    };
    let path = PathBuf::from(path);
    registry::activate();
    registry::set_main_window(window.window_handle().window_id());
    match server::spawn(&path) {
        Ok((server, rx)) => {
            tracing::info!(target: "oino_automation", socket = %server.path().display(),
                "automation listening");
            cx.spawn_in(window, async move |this, cx| {
                serve(this, cx, server, rx).await;
            })
            .detach();
        }
        Err(e) => tracing::error!(target: "oino_automation", %e,
            path = %path.display(), "automation socket"),
    }
}

/// Answers requests until the client closes the connection or the app quits.
async fn serve(
    this: WeakEntity<AppShell>,
    cx: &mut AsyncWindowContext,
    _server: Server,
    mut rx: mpsc::UnboundedReceiver<Call>,
) {
    while let Some(call) = rx.recv().await {
        let Call { request, reply } = call;
        let method = request.method.clone();
        let reply_value = handle(&this, cx, request).await;
        tracing::debug!(target: "oino_automation", %method, %reply_value, "handled");
        if reply.send(reply_value).is_err() || this.upgrade().is_none() {
            break;
        }
    }
}

/// Answers one request.
async fn handle(
    this: &WeakEntity<AppShell>,
    cx: &mut AsyncWindowContext,
    request: Request,
) -> Value {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "ping" => Ok(json!({"pong": true, "version": env!("CARGO_PKG_VERSION")})),
        "tree" => Ok(tree(cx, &request)),
        "state" => read_state(this, cx),
        "click" | "double_click" | "right_click" | "hover" => pointer(cx, &request).await,
        "scroll" => scroll(cx, &request).await,
        "type_text" => type_text(cx, &request).await,
        "key" => key(cx, &request).await,
        "focus" => focus(cx, &request).await,
        "wait_for" => wait_for(cx, &request).await,
        "screenshot" => screenshot(cx, &request).await,
        "drop" => drop_paths(cx, &request).await,
        "clipboard_image" => clipboard_image(cx, &request).await,
        "settle" => settle(cx, 2, SETTLE_TIMEOUT)
            .await
            .map(|()| json!({"frame": registry::frame_number()})),
        "quit" => {
            let _ = cx.update(|_, cx| cx.quit());
            Ok(json!({"quitting": true}))
        }
        other => Err(format!("unknown method {other:?}")),
    };
    match result {
        Ok(value) => ok(&id, &value),
        Err(message) => error(&id, message),
    }
}

/// `tree`: the semantic element list of the last finished frame.
fn tree(cx: &mut AsyncWindowContext, request: &Request) -> Value {
    let selector = request.selector();
    let filtered = request.has_selector();
    let nodes = registry::snapshot();
    let elements: Vec<Value> = nodes
        .iter()
        .filter(|n| !filtered || selector.matches(n))
        .filter(|n| !filtered || selector.include_hidden || n.visible)
        .map(Node::to_json)
        .collect();
    let size = cx
        .update(|window: &mut Window, _: &mut App| {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        })
        .unwrap_or((0.0, 0.0));
    json!({
        "elements": elements,
        "frame": registry::frame_number(),
        "window": {"width": size.0, "height": size.1},
    })
}

/// `state`: screen, session id, pending dialogs.
fn read_state(this: &WeakEntity<AppShell>, cx: &mut AsyncWindowContext) -> Result<Value, String> {
    let mut value = this
        .update(cx, |ws, cx| ws.automation_state(cx).to_json())
        .map_err(|_| "the window is gone".to_owned())?;
    if let Some(object) = value.as_object_mut() {
        object.insert("frame".into(), json!(registry::frame_number()));
    }
    Ok(value)
}

/// Resolves the target of a request: `(x, y, node id_or_null)`.
fn target(request: &Request) -> Result<(f32, f32, Option<String>), String> {
    if let (Some(x), Some(y)) = (request.num("x"), request.num("y")) {
        return Ok((num_f32(x)?, num_f32(y)?, None));
    }
    if !request.has_selector() {
        return Err("give either \"id\" / \"label\" / \"role\" or \"x\" and \"y\"".into());
    }
    let selector = request.selector();
    let nodes = registry::snapshot();
    let node = registry::select(&nodes, &selector)?;
    let (x, y) = node.center();
    Ok((x, y, Some(node.id.clone())))
}

/// Re-resolves a selector against the frame that will hit-test the injected events and
/// returns its fresh centre (QA-r4 H-3): the settle frames that precede a pointer request
/// can re-lay out the window — a late row insertion right after `busy` clears shifts a
/// bottom-pinned transcript — so coordinates resolved before the settle can point where the
/// target *was*, and the synthesized MouseDown/Up miss it.
fn recentered(selector: &super::protocol::Selector) -> Result<(f32, f32, String), String> {
    let nodes = registry::snapshot();
    let node = registry::select(&nodes, selector)?;
    let (x, y) = node.center();
    Ok((x, y, node.id.clone()))
}

/// A `f64` from the protocol as `f32`.
fn num_f32(value: f64) -> Result<f32, String> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "window coordinates are small and the protocol is f64"
    )]
    let out = value as f32;
    if out.is_finite() {
        Ok(out)
    } else {
        Err(format!("{value} is not a coordinate"))
    }
}

/// `click` / `double_click` / `right_click` / `hover`: a real pointer sequence at the target.
async fn pointer(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let selector = request.has_selector().then(|| request.selector());
    let (mut x, mut y, mut id) = target(request)?;
    let method = request.method.as_str();
    let (button, clicks) = match method {
        "right_click" => (MouseButton::Right, 1),
        "double_click" => (MouseButton::Left, 2),
        _ => (MouseButton::Left, 1),
    };
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    if let Some(sel) = &selector {
        let (nx, ny, nid) = recentered(sel)?;
        x = nx;
        y = ny;
        id = Some(nid);
    }
    let hover_only = method == "hover";
    // Each event gets a frame of its own (QA-r4 H-3): dispatching Move+Down+Up in one batch
    // hit-tests them all against the frame painted before the move — when the target just
    // moved (hover bar opening, a row landing at turn end), that frame's hitboxes can still
    // describe the old layout and the synthesized click vanishes on a row with no handler.
    // A person moves, a frame paints the new hover state, then presses; spreading the
    // sequence over frames gives GPUI the same ordering.
    cx.update(|window: &mut Window, cx: &mut App| {
        window.dispatch_event(input::mouse_move(x, y), cx);
    })
    .map_err(|_| "the window is gone".to_owned())?;
    if !hover_only {
        for n in 1..=clicks {
            settle(cx, 1, SETTLE_TIMEOUT).await?;
            cx.update(|window: &mut Window, cx: &mut App| {
                window.dispatch_event(input::mouse_down(x, y, button, n), cx);
            })
            .map_err(|_| "the window is gone".to_owned())?;
            settle(cx, 1, SETTLE_TIMEOUT).await?;
            cx.update(|window: &mut Window, cx: &mut App| {
                window.dispatch_event(input::mouse_up(x, y, button, n), cx);
            })
            .map_err(|_| "the window is gone".to_owned())?;
        }
    }
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"at": [x, y], "id": id, "handled": true}))
}

/// `scroll`: wheel lines at the target (positive `dy` scrolls down).
async fn scroll(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let selector = request.has_selector().then(|| request.selector());
    let (mut x, mut y, mut id) = target(request)?;
    let dy = request.num("dy").map_or(Ok(0.0), num_f32)?;
    let dx = request.num("dx").map_or(Ok(0.0), num_f32)?;
    if dx == 0.0 && dy == 0.0 {
        return Err("give \"dy\" (or \"dx\") lines to scroll".into());
    }
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    if let Some(sel) = &selector {
        let (nx, ny, nid) = recentered(sel)?;
        x = nx;
        y = ny;
        id = Some(nid);
    }
    cx.update(|window: &mut Window, cx: &mut App| {
        window.dispatch_event(input::mouse_move(x, y), cx);
        window.dispatch_event(input::scroll(x, y, dx, dy), cx);
    })
    .map_err(|_| "the window is gone".to_owned())?;
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"at": [x, y], "dx": dx, "dy": dy, "id": id}))
}

/// `type_text`: one keystroke per character into whatever holds keyboard focus.
async fn type_text(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let text = request
        .str("text")
        .ok_or_else(|| "give \"text\" to type".to_owned())?
        .to_owned();
    let submit = request.bool("submit");
    let keys = input::keystrokes_for(&text);
    let typed = keys.len();
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    cx.update(|window: &mut Window, cx: &mut App| {
        for key in keys {
            window.dispatch_keystroke(key, cx);
        }
        if submit {
            let os = cx.platform_info().os;
            if let Ok(enter) = input::keystroke("enter", os) {
                window.dispatch_keystroke(enter, cx);
            }
        }
    })
    .map_err(|_| "the window is gone".to_owned())?;
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"typed": typed, "chars": text.chars().count(), "submit": submit}))
}

/// `key`: one keystroke (`enter`, `tab`, `secondary-k`, `shift-secondary-p`, …).
async fn key(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let spec = request
        .str("key")
        .ok_or_else(|| "give \"key\" (e.g. \"enter\", \"secondary-k\")".to_owned())?
        .to_owned();
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    let dispatched = cx
        .update(|window: &mut Window, cx: &mut App| {
            let os = cx.platform_info().os;
            let keystroke = input::keystroke(&spec, os)?;
            Ok::<bool, String>(window.dispatch_keystroke(keystroke, cx))
        })
        .map_err(|_| "the window is gone".to_owned())??;
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"key": spec, "handled": dispatched}))
}

/// `focus`: keyboard focus to an element that registered a focus handle (inputs, composer).
async fn focus(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    if !request.has_selector() {
        return Err("give \"id\" (or \"label\") of the element to focus".into());
    }
    let selector = request.selector();
    let nodes = registry::snapshot();
    let node = registry::select(&nodes, &selector)?;
    let path = node.path.clone();
    let id = node.id.clone();
    let focused = cx
        .update(|window: &mut Window, cx: &mut App| state::focus_path(window, cx, &path))
        .map_err(|_| "the window is gone".to_owned())?;
    if !focused {
        return Err(format!(
            "{id} has no focus handle (click it instead; only inputs register one)"
        ));
    }
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"focused": id}))
}

/// `wait_for`: polls the element list until the predicate holds or the timeout expires.
async fn wait_for(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let timeout_ms = request.num("timeout_ms").unwrap_or(5000.0).max(0.0);
    let timeout = Duration::from_secs_f64(timeout_ms / 1000.0);
    let absent = request.bool("absent");
    let text = request.str("text").map(str::to_owned);
    let selector = request.selector();
    let by_element = request.has_selector();
    if !by_element && text.is_none() {
        return Err("give \"id\" / \"role\" / \"label\" or \"text\" to wait for".into());
    }
    let started = Instant::now();
    loop {
        let nodes = registry::snapshot();
        let matched = if by_element {
            registry::select(&nodes, &selector).is_ok()
        } else {
            let needle = text.as_deref().unwrap_or_default();
            nodes.iter().any(|n| n.visible && n.contains(needle))
        };
        if matched != absent {
            return Ok(json!({
                "matched": matched,
                "ms": started.elapsed().as_millis(),
                "frame": registry::frame_number(),
            }));
        }
        if started.elapsed() >= timeout {
            let visible = nodes.iter().filter(|n| n.visible).count();
            let mut seen: Vec<String> = nodes
                .iter()
                .filter(|n| n.visible && !n.label.is_empty())
                .map(|n| n.label.as_str())
                .take(12)
                .map(str::to_owned)
                .collect();
            if selector.role.is_some() {
                let rows: Vec<String> = nodes
                    .iter()
                    .filter(|n| n.visible && selector.matches_role(n))
                    .map(|n| format!("{}={}", n.id, n.label))
                    .collect();
                seen.push(format!("[role rows: {}]", rows.join(",")));
            }
            return Err(format!(
                "timed out after {} ms waiting for {} ({} visible elements; labels: {})",
                started.elapsed().as_millis(),
                match (by_element, text.as_deref()) {
                    (true, _) => format!("{selector:?}"),
                    (false, Some(t)) => format!("text {t:?}"),
                    _ => "?".to_owned(),
                },
                visible,
                seen.join(" | "),
            ));
        }
        let _ = cx.update(|window, _| window.refresh());
        let timer = cx.background_executor().timer(Duration::from_millis(25));
        timer.await;
    }
}

/// `drop`: files dragged onto the window at the target. The platform reports a drag as
/// `FileDropEvent` (entered → submitted) and GPUI turns it into the pointer sequence that hits
/// the `on_drop` target, so this is the same path as dragging from a file manager.
async fn drop_paths(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let paths: Vec<PathBuf> = request
        .strings("paths")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        return Err("give \"paths\": [\"/path/to/file.png\"]".into());
    }
    // A drag of a missing file is a silent no-op downstream (QA-r4 H-4): fail with the path
    // instead, so a scenario written for another host says so instead of timing out later.
    if let Some(missing) = paths.iter().find(|p| !p.is_file()) {
        return Err(format!(
            "drop path {} is not a file on the app host",
            missing.display()
        ));
    }
    let selector = request.has_selector().then(|| request.selector());
    let (mut x, mut y, mut id) =
        if request.has_selector() || (request.num("x").is_some() && request.num("y").is_some()) {
            target(request)?
        } else {
            // No target: the centre of the window.
            let (x, y) = cx
                .update(|window: &mut Window, _| {
                    let size = window.viewport_size();
                    (f32::from(size.width) / 2.0, f32::from(size.height) / 2.0)
                })
                .map_err(|_| "the window is gone".to_owned())?;
            (x, y, None)
        };
    let dropped = paths.clone();
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    if let Some(sel) = &selector {
        // Same race as `pointer` (H-3): drop where the target is in the frame that will
        // hit-test the drag, not where it was before the settle frames.
        let (nx, ny, nid) = recentered(sel)?;
        x = nx;
        y = ny;
        id = Some(nid);
    }
    cx.update(move |window: &mut Window, cx: &mut App| {
        // A mouse move first: a `FileDrop` event does not switch the input modality, and hover
        // (which the drop listeners test) is suppressed while the last input was a keystroke.
        window.dispatch_event(input::mouse_move(x, y), cx);
        let position = gpui_kit::point(gpui_kit::px(x), gpui_kit::px(y));
        let entered = gpui_kit::FileDropEvent::Entered {
            position,
            paths: gpui_kit::ExternalPaths(dropped.into_iter().collect()),
        };
        window.dispatch_event(PlatformInput::FileDrop(entered), cx);
        window.dispatch_event(
            PlatformInput::FileDrop(gpui_kit::FileDropEvent::Submit { position }),
            cx,
        );
    })
    .map_err(|_| "the window is gone".to_owned())?;
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({
        "at": [x, y],
        "id": id,
        "paths": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    }))
}

/// `clipboard_image`: puts a PNG from the app host on the clipboard, so `secondary-v` in the
/// composer pastes it exactly like an image copied from another app.
async fn clipboard_image(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let path = request
        .str("path")
        .ok_or_else(|| "give \"path\" of a PNG on the app host".to_owned())?
        .to_owned();
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {path}: {e}"))?;
    let len = bytes.len();
    cx.update(move |_, cx: &mut App| {
        let image = gpui_kit::Image {
            format: gpui_kit::ImageFormat::Png,
            bytes,
            id: u64::try_from(len).unwrap_or(0),
        };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&image));
    })
    .map_err(|_| "the window is gone".to_owned())?;
    settle(cx, 1, SETTLE_TIMEOUT).await?;
    Ok(json!({"path": path, "bytes": len}))
}

/// `screenshot`: a PNG of the app window (X11 `import`, `xdotool` for the window id).
async fn screenshot(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let path = request.str("path").map_or_else(
        || std::env::temp_dir().join(format!("oino-shot-{}.png", registry::frame_number())),
        PathBuf::from,
    );
    let display = std::env::var("DISPLAY").ok();
    let out = path.clone();
    let result = cx
        .background_spawn(async move { capture(&out, display.as_deref()) })
        .await;
    result.map(|size| {
        json!({
            "path": path.display().to_string(),
            "width": size.0,
            "height": size.1,
        })
    })
}

/// Runs `import` for the app window; returns its pixel size when `identify` could read it.
/// Hosts without [`ImageMagick`] fall back to `ffmpeg -f x11grab` of the whole display (the
/// driver runs on a private Xvfb that shows only the app).
///
/// [`ImageMagick`]: https://imagemagick.org
fn capture(path: &Path, display: Option<&str>) -> Result<(Option<u32>, Option<u32>), String> {
    let mut import = std::process::Command::new("import");
    if let Some(display) = display {
        import.args(["-display", display]);
    }
    let window = xdotool_window(display).unwrap_or_else(|| "root".to_owned());
    match import.args(["-window", &window]).arg(path).output() {
        Ok(output) if output.status.success() => Ok(identify(path).unwrap_or((None, None))),
        Ok(output) => Err(format!(
            "import failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(spawn) if spawn.kind() == std::io::ErrorKind::NotFound => {
            ffmpeg_capture(path, display).or_else(|_| pil_capture(path, display))
        }
        Err(e) => Err(format!("running import: {e} (ImageMagick is required)")),
    }
}

/// `PIL.ImageGrab.grab(xdisplay=…)` of the whole display, one frame (the last capture
/// fallback: `import` and an `x11grab`-capable ffmpeg may both be missing on a dev box).
fn pil_capture(path: &Path, display: Option<&str>) -> Result<(Option<u32>, Option<u32>), String> {
    let fallback_display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_owned());
    let display = display.unwrap_or(&fallback_display);
    let script = format!(
        "from PIL import ImageGrab; im = ImageGrab.grab(xdisplay='{display}'); \
         im.save('{}'); print(im.size)",
        path.display()
    );
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .map_err(|e| format!("running python3/PIL: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "python3 PIL grab failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let dims = text
        .trim()
        .strip_prefix('(')
        .and_then(|t| t.strip_suffix(')'))
        .and_then(|t| t.split_once(", "))
        .ok_or_else(|| format!("PIL printed {text:?}, expected (w, h)"))?;
    let parse = |v: &str, what: &str| v.parse().map_err(|_| format!("PIL {what} in {text:?}"));
    let (w, h) = (parse(dims.0, "width")?, parse(dims.1, "height")?);
    Ok((Some(w), Some(h)))
}

/// `ffmpeg -f x11grab` of the whole display (one frame).
fn ffmpeg_capture(
    path: &Path,
    display: Option<&str>,
) -> Result<(Option<u32>, Option<u32>), String> {
    let fallback_display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_owned());
    let display = display.unwrap_or(&fallback_display);
    let size = std::process::Command::new("xdpyinfo")
        .arg("-display")
        .arg(display)
        .output()
        .ok()
        .and_then(|out| {
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text.lines().find(|l| l.contains("dimensions:"))?;
            let dims = line.split_whitespace().nth(2)?.to_owned();
            dims.contains('x').then_some(dims)
        })
        .unwrap_or_else(|| "1600x1000".to_owned());
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "x11grab",
            "-framerate",
            "1",
        ])
        .args(["-video_size", &size])
        .arg("-i")
        .arg(display)
        .args(["-frames:v", "1"])
        .arg(path)
        .output()
        .map_err(|e| format!("running ffmpeg: {e} (ImageMagick `import` or ffmpeg is required)"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg x11grab failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(identify(path).unwrap_or((None, None)))
}

/// The X11 window id of the app window (`xdotool search`).
fn xdotool_window(display: Option<&str>) -> Option<String> {
    let mut cmd = std::process::Command::new("xdotool");
    if let Some(display) = display {
        cmd.args(["--display", display]);
    }
    let output = cmd
        .args(["search", "--onlyvisible", "--class", "dev.oino.desktop"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|l| l.trim().to_owned())
}

/// The PNG's pixel size (`identify -format %wx%h`).
fn identify(path: &Path) -> Option<(Option<u32>, Option<u32>)> {
    let output = std::process::Command::new("identify")
        .args(["-format", "%w %h"])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (w, h) = text.trim().split_once(' ')?;
    Some((w.parse().ok(), h.parse().ok()))
}

/// Lets `frames` frames pass (a forced refresh plus `on_next_frame`), bounded by `timeout`.
async fn settle(cx: &mut AsyncWindowContext, frames: u32, timeout: Duration) -> Result<(), String> {
    for _ in 0..frames {
        let (tx, rx) = futures::channel::oneshot::channel::<()>();
        let mut tx = Some(tx);
        cx.on_next_frame(move |_window, _cx| {
            if let Some(tx) = tx.take() {
                let _ = tx.send(());
            }
        });
        let _ = cx.update(|window, _| window.refresh());
        let timer = cx.background_executor().timer(timeout);
        let _ = futures::future::select(rx, timer).await;
    }
    Ok(())
}
