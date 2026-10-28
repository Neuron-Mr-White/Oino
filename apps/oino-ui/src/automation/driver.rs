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
    let (x, y, id) = target(request)?;
    let method = request.method.as_str();
    let (button, clicks) = match method {
        "right_click" => (MouseButton::Right, 1),
        "double_click" => (MouseButton::Left, 2),
        _ => (MouseButton::Left, 1),
    };
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    let hover_only = method == "hover";
    let events: Vec<PlatformInput> = if hover_only {
        vec![input::mouse_move(x, y)]
    } else {
        let mut events = vec![input::mouse_move(x, y)];
        for n in 1..=clicks {
            events.push(input::mouse_down(x, y, button, n));
            events.push(input::mouse_up(x, y, button, n));
        }
        events
    };
    cx.update(|window: &mut Window, cx: &mut App| {
        for event in events {
            window.dispatch_event(event, cx);
        }
    })
    .map_err(|_| "the window is gone".to_owned())?;
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
    Ok(json!({"at": [x, y], "id": id, "handled": true}))
}

/// `scroll`: wheel lines at the target (positive `dy` scrolls down).
async fn scroll(cx: &mut AsyncWindowContext, request: &Request) -> Result<Value, String> {
    let (x, y, id) = target(request)?;
    let dy = request.num("dy").map_or(Ok(0.0), num_f32)?;
    let dx = request.num("dx").map_or(Ok(0.0), num_f32)?;
    if dx == 0.0 && dy == 0.0 {
        return Err("give \"dy\" (or \"dx\") lines to scroll".into());
    }
    settle(cx, SETTLE_FRAMES, SETTLE_TIMEOUT).await?;
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
            let seen: Vec<&str> = nodes
                .iter()
                .filter(|n| n.visible && !n.label.is_empty())
                .map(|n| n.label.as_str())
                .take(12)
                .collect();
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
fn capture(path: &Path, display: Option<&str>) -> Result<(Option<u32>, Option<u32>), String> {
    let mut import = std::process::Command::new("import");
    if let Some(display) = display {
        import.args(["-display", display]);
    }
    let window = xdotool_window(display).unwrap_or_else(|| "root".to_owned());
    let output = import
        .args(["-window", &window])
        .arg(path)
        .output()
        .map_err(|e| format!("running import: {e} (ImageMagick is required)"))?;
    if !output.status.success() {
        return Err(format!(
            "import failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let size = identify(path).unwrap_or((None, None));
    Ok(size)
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
