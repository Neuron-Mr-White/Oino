//! Human-like input plans for the CDP relay (UNI-293): Bézier mouse paths
//! with natural timing and overshoot, per-character typing with human
//! inter-key delays, mistypes and Shift handling, and eased wheel scrolling.
//!
//! Ported from CloakBrowser's MIT-licensed wrapper (npm `cloakbrowser`
//! 0.6.2, `dist/human/{config,mouse,keyboard}.js` and
//! `dist/human-puppeteer/scroll.js`, <https://github.com/CloakHQ/CloakBrowser>):
//!
//! > MIT License — Copyright (c) 2026 CloakHQ
//! >
//! > Permission is hereby granted, free of charge, to any person obtaining a copy
//! > of this software and associated documentation files (the "Software"), to deal
//! > in the Software without restriction, including without limitation the rights
//! > to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! > copies of the Software, and to permit persons to whom the Software is
//! > furnished to do so, subject to the following conditions:
//! >
//! > The above copyright notice and this permission notice shall be included in all
//! > copies or substantial portions of the Software.
//! >
//! > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! > IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! > FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! > AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! > LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! > SOFTWARE.
//!
//! Only the binary of CloakBrowser is under its restrictive licence; this is
//! the wrapper's (MIT) logic, re-written in Rust. Differences from the
//! wrapper are marked "unipi:" below.
//!
//! Everything here is pure (plans, no I/O) so it can be unit-tested; the
//! relay ([`super::gate`]) executes the plans over CDP.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// `browser.humanize` in config.json: the wrapper's two presets, plus
/// `off` (config only — the CLI exposes no off switch).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Humanize {
    #[default]
    Default,
    Careful,
    Off,
}

impl Humanize {
    pub fn as_str(self) -> &'static str {
        match self {
            Humanize::Default => "default",
            Humanize::Careful => "careful",
            Humanize::Off => "off",
        }
    }
    pub fn is_default(&self) -> bool {
        *self == Humanize::Default
    }
    /// The preset's numbers (`None` = pass input through untouched).
    pub fn config(self) -> Option<HumanConfig> {
        match self {
            Humanize::Default => Some(HumanConfig::default_preset()),
            Humanize::Careful => Some(HumanConfig::careful()),
            Humanize::Off => None,
        }
    }
}

type Range = (f64, f64);

/// The wrapper's `HumanConfig` (milliseconds / CSS pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct HumanConfig {
    pub typing_delay: f64,
    pub typing_delay_spread: f64,
    pub typing_pause_chance: f64,
    pub typing_pause_range: Range,
    pub shift_down_delay: Range,
    pub shift_up_delay: Range,
    pub key_hold: Range,
    pub field_switch_delay: Range,
    pub mistype_chance: f64,
    pub mistype_delay_notice: Range,
    pub mistype_delay_correct: Range,
    pub mouse_steps_divisor: f64,
    pub mouse_min_steps: u32,
    pub mouse_max_steps: u32,
    pub mouse_wobble_max: f64,
    pub mouse_overshoot_chance: f64,
    pub mouse_overshoot_px: Range,
    pub mouse_burst_size: (u32, u32),
    pub mouse_burst_pause: Range,
    pub click_aim_delay_input: Range,
    pub click_aim_delay_button: Range,
    pub click_hold_input: Range,
    pub click_hold_button: Range,
    pub click_input_x_range: Range,
    pub scroll_delta_base: Range,
    pub scroll_delta_variance: f64,
    pub scroll_pause_fast: Range,
    pub scroll_pause_slow: Range,
    pub scroll_accel_steps: (u32, u32),
    pub scroll_decel_steps: (u32, u32),
    pub scroll_overshoot_chance: f64,
    pub scroll_overshoot_px: Range,
    pub scroll_settle_delay: Range,
    pub scroll_target_zone: Range,
    pub scroll_pre_move_delay: Range,
    pub initial_cursor_x: Range,
    pub initial_cursor_y: Range,
}

impl HumanConfig {
    /// The wrapper's `default` preset.
    pub fn default_preset() -> HumanConfig {
        HumanConfig {
            typing_delay: 70.0,
            typing_delay_spread: 40.0,
            typing_pause_chance: 0.1,
            typing_pause_range: (400.0, 1000.0),
            shift_down_delay: (30.0, 70.0),
            shift_up_delay: (20.0, 50.0),
            key_hold: (15.0, 35.0),
            field_switch_delay: (800.0, 1500.0),
            mistype_chance: 0.02,
            mistype_delay_notice: (100.0, 300.0),
            mistype_delay_correct: (50.0, 150.0),
            mouse_steps_divisor: 8.0,
            mouse_min_steps: 25,
            mouse_max_steps: 80,
            mouse_wobble_max: 1.5,
            mouse_overshoot_chance: 0.15,
            mouse_overshoot_px: (3.0, 6.0),
            mouse_burst_size: (3, 5),
            mouse_burst_pause: (8.0, 18.0),
            click_aim_delay_input: (60.0, 140.0),
            click_aim_delay_button: (80.0, 200.0),
            click_hold_input: (40.0, 100.0),
            click_hold_button: (60.0, 150.0),
            click_input_x_range: (0.05, 0.30),
            scroll_delta_base: (80.0, 130.0),
            scroll_delta_variance: 0.2,
            scroll_pause_fast: (30.0, 80.0),
            scroll_pause_slow: (80.0, 200.0),
            scroll_accel_steps: (2, 3),
            scroll_decel_steps: (2, 3),
            scroll_overshoot_chance: 0.1,
            scroll_overshoot_px: (50.0, 150.0),
            scroll_settle_delay: (300.0, 600.0),
            scroll_target_zone: (0.20, 0.80),
            scroll_pre_move_delay: (100.0, 300.0),
            initial_cursor_x: (400.0, 700.0),
            initial_cursor_y: (45.0, 60.0),
        }
    }

    /// The wrapper's `careful` preset: everything slower.
    pub fn careful() -> HumanConfig {
        HumanConfig {
            typing_delay: 100.0,
            typing_delay_spread: 50.0,
            typing_pause_chance: 0.15,
            typing_pause_range: (500.0, 1200.0),
            shift_down_delay: (40.0, 90.0),
            shift_up_delay: (30.0, 70.0),
            key_hold: (20.0, 45.0),
            field_switch_delay: (1000.0, 2000.0),
            mistype_chance: 0.03,
            mistype_delay_notice: (150.0, 400.0),
            mistype_delay_correct: (80.0, 200.0),
            mouse_overshoot_chance: 0.10,
            mouse_burst_pause: (12.0, 25.0),
            click_aim_delay_input: (80.0, 180.0),
            click_aim_delay_button: (120.0, 280.0),
            click_hold_input: (60.0, 140.0),
            click_hold_button: (80.0, 200.0),
            scroll_pause_fast: (100.0, 200.0),
            scroll_pause_slow: (250.0, 600.0),
            scroll_settle_delay: (400.0, 800.0),
            scroll_pre_move_delay: (150.0, 400.0),
            ..HumanConfig::default_preset()
        }
    }
}

/// Random float in `[lo, hi]`.
pub fn rand_range<R: Rng + ?Sized>(rng: &mut R, (lo, hi): Range) -> f64 {
    if hi <= lo {
        lo
    } else {
        rng.random_range(lo..=hi)
    }
}

fn rand_int<R: Rng + ?Sized>(rng: &mut R, (lo, hi): (u32, u32)) -> u32 {
    if hi <= lo {
        lo
    } else {
        rng.random_range(lo..=hi)
    }
}

// ------------------------------------------------------------------ mouse

/// The wrapper's cubic ease-in-out.
pub fn ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn bezier(p: [(f64, f64); 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0, a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1)
}

/// One `mouseMoved` of a path: dispatched `wait_ms` after the previous one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStep {
    pub x: f64,
    pub y: f64,
    pub wait_ms: f64,
}

/// A human mouse path from `from` to `to` (the wrapper's `humanMove`):
/// a cubic Bézier with control points biased up to 30 % of the distance
/// sideways, eased in/out, a sine-shaped wobble, motion in bursts with short
/// pauses, and sometimes an overshoot of a few pixels and a correction.
///
/// unipi: points are dispatched 2–6 ms apart inside a burst (a ~200 Hz mouse,
/// not back-to-back), a move never has more steps than pixels, and the last
/// point is exactly `to` (the click point was already randomised).
pub fn mouse_path<R: Rng + ?Sized>(from: (f64, f64), to: (f64, f64), cfg: &HumanConfig, rng: &mut R) -> Vec<MoveStep> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let dist = dx.hypot(dy);
    if dist < 1.0 {
        return if dist > 0.0 { vec![MoveStep { x: to.0, y: to.1, wait_ms: 0.0 }] } else { Vec::new() };
    }
    let steps = ((dist / cfg.mouse_steps_divisor).round() as u32)
        .clamp(cfg.mouse_min_steps, cfg.mouse_max_steps)
        .min(dist.round().max(1.0) as u32)
        .max(1);
    let (px, py) = (-dy / dist, dx / dist);
    let bias1 = rng.random_range(-0.3..=0.3) * dist;
    let bias2 = rng.random_range(-0.3..=0.3) * dist;
    let cp1 = (from.0 + dx * 0.25 + px * bias1, from.1 + dy * 0.25 + py * bias1);
    let cp2 = (from.0 + dx * 0.75 + px * bias2, from.1 + dy * 0.75 + py * bias2);
    let burst = rand_int(rng, cfg.mouse_burst_size).max(1);
    let mut out = Vec::with_capacity(steps as usize + 2);
    let mut in_burst = 0;
    for i in 1..=steps {
        let progress = i as f64 / steps as f64;
        let (x, y) = bezier([from, cp1, cp2, to], ease_in_out(progress));
        let amp = (std::f64::consts::PI * progress).sin() * cfg.mouse_wobble_max;
        let (x, y) = if i == steps {
            to
        } else {
            ((x + rng.random_range(-1.0..=1.0) * amp).round(), (y + rng.random_range(-1.0..=1.0) * amp).round())
        };
        let mut wait = rng.random_range(2.0..=6.0);
        if in_burst >= burst {
            wait += rand_range(rng, cfg.mouse_burst_pause);
            in_burst = 0;
        }
        in_burst += 1;
        out.push(MoveStep { x, y, wait_ms: wait });
    }
    if rng.random_bool(cfg.mouse_overshoot_chance.clamp(0.0, 1.0)) {
        let over = rand_range(rng, cfg.mouse_overshoot_px);
        let angle = dy.atan2(dx);
        let last = out.pop().expect("at least one step");
        out.push(MoveStep { x: (to.0 + angle.cos() * over).round(), y: (to.1 + angle.sin() * over).round(), wait_ms: last.wait_ms });
        out.push(MoveStep { x: to.0, y: to.1, wait_ms: rng.random_range(30.0..=70.0) });
    }
    out
}

/// An element's box (CSS px, viewport coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BoxRect {
    /// From a CDP quad `[x1,y1,…,x4,y4]`.
    pub fn from_quad(quad: &[f64]) -> Option<BoxRect> {
        if quad.len() < 8 {
            return None;
        }
        let xs = [quad[0], quad[2], quad[4], quad[6]];
        let ys = [quad[1], quad[3], quad[5], quad[7]];
        let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
        let (y0, y1) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
        Some(BoxRect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// Where a person clicks in `rect` (the wrapper's `clickTarget`): text
/// fields near their left (where the text starts), anything else somewhere
/// around the middle — never the exact centre.
pub fn click_target<R: Rng + ?Sized>(rect: &BoxRect, is_input: bool, cfg: &HumanConfig, rng: &mut R) -> (f64, f64) {
    let (fx, fy) = if is_input {
        (rand_range(rng, cfg.click_input_x_range), rng.random_range(0.30..=0.70))
    } else {
        (rng.random_range(0.35..=0.65), rng.random_range(0.35..=0.65))
    };
    ((rect.x + rect.width * fx).round(), (rect.y + rect.height * fy).round())
}

// --------------------------------------------------------------- keyboard

/// Characters typed with Shift held (US layout), as in the wrapper.
pub const SHIFT_SYMBOLS: &str = "@#!$%^&*()_+{}|:\"<>?~";

/// Neighbouring keys for a mistype (letters/digits only, as in the wrapper).
fn nearby(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase() {
        'a' => "sqwz", 'b' => "vghn", 'c' => "xdfv", 'd' => "sfecx", 'e' => "wrsdf",
        'f' => "dgrtcv", 'g' => "fhtyb", 'h' => "gjybn", 'i' => "ujko", 'j' => "hkunm",
        'k' => "jloi", 'l' => "kop", 'm' => "njk", 'n' => "bhjm", 'o' => "iklp",
        'p' => "ol", 'q' => "wa", 'r' => "edft", 's' => "awedxz", 't' => "rfgy",
        'u' => "yhji", 'v' => "cfgb", 'w' => "qase", 'x' => "zsdc", 'y' => "tghu",
        'z' => "asx",
        '1' => "2", '2' => "13", '3' => "24", '4' => "35", '5' => "46",
        '6' => "57", '7' => "68", '8' => "79", '9' => "80", '0' => "9",
        _ => return None,
    })
}

/// The physical key of a printable ASCII char: `(code, windows vk, shift)`.
pub fn key_of(c: char) -> Option<(String, u32, bool)> {
    let shifted = |code: &str, vk: u32| Some((code.to_string(), vk, true));
    let plain = |code: &str, vk: u32| Some((code.to_string(), vk, false));
    match c {
        'a'..='z' => Some((format!("Key{}", c.to_ascii_uppercase()), c.to_ascii_uppercase() as u32, false)),
        'A'..='Z' => Some((format!("Key{c}"), c as u32, true)),
        '0'..='9' => Some((format!("Digit{c}"), c as u32, false)),
        ' ' => plain("Space", 32),
        '-' => plain("Minus", 189),
        '=' => plain("Equal", 187),
        '[' => plain("BracketLeft", 219),
        ']' => plain("BracketRight", 221),
        '\\' => plain("Backslash", 220),
        ';' => plain("Semicolon", 186),
        '\'' => plain("Quote", 222),
        ',' => plain("Comma", 188),
        '.' => plain("Period", 190),
        '/' => plain("Slash", 191),
        '`' => plain("Backquote", 192),
        '!' => shifted("Digit1", 49),
        '@' => shifted("Digit2", 50),
        '#' => shifted("Digit3", 51),
        '$' => shifted("Digit4", 52),
        '%' => shifted("Digit5", 53),
        '^' => shifted("Digit6", 54),
        '&' => shifted("Digit7", 55),
        '*' => shifted("Digit8", 56),
        '(' => shifted("Digit9", 57),
        ')' => shifted("Digit0", 48),
        '_' => shifted("Minus", 189),
        '+' => shifted("Equal", 187),
        '{' => shifted("BracketLeft", 219),
        '}' => shifted("BracketRight", 221),
        '|' => shifted("Backslash", 220),
        ':' => shifted("Semicolon", 186),
        '"' => shifted("Quote", 222),
        '<' => shifted("Comma", 188),
        '>' => shifted("Period", 190),
        '?' => shifted("Slash", 191),
        '~' => shifted("Backquote", 192),
        _ => None,
    }
}

/// One step of a typing plan, dispatched `wait_ms` after the previous one.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyStep {
    /// `Input.dispatchKeyEvent` params.
    Key { params: Value, wait_ms: f64 },
    /// `Input.insertText` (non-ASCII, control chars: what an IME / paste gives).
    Text { text: String, wait_ms: f64 },
}

impl KeyStep {
    pub fn wait_ms(&self) -> f64 {
        match self {
            KeyStep::Key { wait_ms, .. } | KeyStep::Text { wait_ms, .. } => *wait_ms,
        }
    }
}

const SHIFT: u32 = 8;

fn shift_event(kind: &str) -> Value {
    json!({ "type": kind, "key": "Shift", "code": "ShiftLeft", "windowsVirtualKeyCode": 16, "nativeVirtualKeyCode": 16,
            "modifiers": if kind == "keyUp" { 0 } else { SHIFT } })
}

fn char_down(c: char, code: &str, vk: u32, shift: bool) -> Value {
    json!({ "type": "keyDown", "key": c.to_string(), "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk,
            "text": c.to_string(), "unmodifiedText": c.to_string(), "modifiers": if shift { SHIFT } else { 0 } })
}

fn char_up(c: char, code: &str, vk: u32, shift: bool) -> Value {
    json!({ "type": "keyUp", "key": c.to_string(), "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk,
            "modifiers": if shift { SHIFT } else { 0 } })
}

/// `keyDown` + `keyUp` of a named key (Backspace, Enter…).
pub fn named_key(key: &str) -> Option<(Value, Value)> {
    let (code, vk, text): (&str, u32, Option<&str>) = match key {
        "Backspace" => ("Backspace", 8, None),
        "Enter" => ("Enter", 13, Some("\r")),
        "Tab" => ("Tab", 9, None),
        "Delete" => ("Delete", 46, None),
        _ => return None,
    };
    let mut down = json!({ "type": if text.is_some() { "keyDown" } else { "rawKeyDown" }, "key": key, "code": code,
                           "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk });
    if let Some(text) = text {
        down["text"] = json!(text);
        down["unmodifiedText"] = json!(text);
    }
    let up = json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk });
    Some((down, up))
}

/// The wrapper's inter-key delay: usually `typing_delay ± spread`,
/// sometimes a thinking pause.
pub fn inter_key_delay<R: Rng + ?Sized>(cfg: &HumanConfig, rng: &mut R) -> f64 {
    if rng.random_bool(cfg.typing_pause_chance.clamp(0.0, 1.0)) {
        rand_range(rng, cfg.typing_pause_range)
    } else {
        (cfg.typing_delay + rng.random_range(-1.0..=1.0) * cfg.typing_delay_spread).max(10.0)
    }
}

fn push_char<R: Rng + ?Sized>(out: &mut Vec<KeyStep>, c: char, code: &str, vk: u32, shift: bool, first_wait: f64, cfg: &HumanConfig, rng: &mut R) {
    if shift {
        out.push(KeyStep::Key { params: shift_event("rawKeyDown"), wait_ms: first_wait });
        out.push(KeyStep::Key { params: char_down(c, code, vk, true), wait_ms: rand_range(rng, cfg.shift_down_delay) });
        out.push(KeyStep::Key { params: char_up(c, code, vk, true), wait_ms: rand_range(rng, cfg.key_hold) });
        out.push(KeyStep::Key { params: shift_event("keyUp"), wait_ms: rand_range(rng, cfg.shift_up_delay) });
    } else {
        out.push(KeyStep::Key { params: char_down(c, code, vk, false), wait_ms: first_wait });
        out.push(KeyStep::Key { params: char_up(c, code, vk, false), wait_ms: rand_range(rng, cfg.key_hold) });
    }
}

/// Typing `text` like a person (the wrapper's `humanType`): printable ASCII
/// as real key presses (Shift for capitals and shifted symbols), human
/// inter-key delays with occasional pauses, an occasional mistype of a
/// letter/digit that is noticed and Backspaced. Non-ASCII (IME text, emoji)
/// and control characters go in with `Input.insertText`, one character at a
/// time, so their meaning never changes (a `\n` doesn't become an Enter that
/// submits a form). The first step waits 0 ms (the caller spaces calls).
pub fn typing_plan<R: Rng + ?Sized>(text: &str, cfg: &HumanConfig, rng: &mut R) -> Vec<KeyStep> {
    let mut out = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let wait = if i == 0 { 0.0 } else { inter_key_delay(cfg, rng) };
        let Some((code, vk, shift)) = key_of(c) else {
            out.push(KeyStep::Text { text: c.to_string(), wait_ms: wait + rand_range(rng, cfg.key_hold) });
            continue;
        };
        let mut wait = wait;
        if c.is_ascii_alphanumeric() && rng.random_bool(cfg.mistype_chance.clamp(0.0, 1.0)) {
            if let Some(near) = nearby(c) {
                let pick = near.chars().nth(rng.random_range(0..near.chars().count())).unwrap_or(c);
                let wrong = if c.is_ascii_uppercase() { pick.to_ascii_uppercase() } else { pick };
                if let Some((wcode, wvk, wshift)) = key_of(wrong) {
                    push_char(&mut out, wrong, &wcode, wvk, wshift, wait, cfg, rng);
                    let (down, up) = named_key("Backspace").expect("backspace");
                    out.push(KeyStep::Key { params: down, wait_ms: rand_range(rng, cfg.mistype_delay_notice) });
                    out.push(KeyStep::Key { params: up, wait_ms: rand_range(rng, cfg.key_hold) });
                    wait = rand_range(rng, cfg.mistype_delay_correct);
                }
            }
        }
        push_char(&mut out, c, &code, vk, shift, wait, cfg, rng);
    }
    out
}

// ------------------------------------------------------------------ wheel

/// One wheel event: `delta` px, dispatched `wait_ms` after the previous one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelStep {
    pub delta: f64,
    pub wait_ms: f64,
}

/// A human scroll of exactly `delta` px on one axis (the wrapper's
/// accelerate → cruise → decelerate "notches", each sent as 20–40 px wheel
/// events 8–20 ms apart — `smoothWheel`), pauses between notches.
///
/// unipi: the notch sizes are scaled so the total is exactly `delta` (an
/// agent's `scroll down 500` scrolls 500 px), and there is no overshoot here
/// (see [`overshoot_plan`] for scrolling to an element).
pub fn wheel_plan<R: Rng + ?Sized>(delta: f64, cfg: &HumanConfig, rng: &mut R) -> Vec<WheelStep> {
    let total = delta.abs().round();
    if total < 1.0 {
        return Vec::new();
    }
    let sign = delta.signum();
    let avg = (cfg.scroll_delta_base.0 + cfg.scroll_delta_base.1) / 2.0;
    let clicks = ((total / avg).ceil() as usize).max(1);
    let accel = rand_int(rng, cfg.scroll_accel_steps) as usize;
    let decel = rand_int(rng, cfg.scroll_decel_steps) as usize;
    let mut notches: Vec<(f64, f64)> = (0..clicks)
        .map(|i| {
            let (size, pause) = if i < accel {
                (rng.random_range(80.0..=100.0), rand_range(rng, cfg.scroll_pause_slow))
            } else if i + decel >= clicks {
                (rng.random_range(60.0..=90.0), rand_range(rng, cfg.scroll_pause_slow))
            } else {
                (rand_range(rng, cfg.scroll_delta_base), rand_range(rng, cfg.scroll_pause_fast))
            };
            (size * (1.0 + rng.random_range(-1.0..=1.0) * cfg.scroll_delta_variance), pause)
        })
        .collect();
    let sum: f64 = notches.iter().map(|n| n.0).sum();
    for notch in &mut notches {
        notch.0 = notch.0 * total / sum;
    }
    // Integer wheel events, the remainder carried so the sum is exact.
    let mut out = Vec::new();
    let mut sent = 0.0;
    let mut target = 0.0;
    for (n, (size, pause)) in notches.iter().enumerate() {
        target += size;
        let goal = if n + 1 == notches.len() { total } else { target.round() };
        let mut first = true;
        while sent < goal {
            let left = goal - sent;
            let mut chunk = rng.random_range(20.0_f64..=40.0).round().min(left);
            if left - chunk < 1.0 {
                chunk = left;
            }
            let wait = if first && n > 0 { *pause } else if out.is_empty() { 0.0 } else { rng.random_range(8.0..=20.0) };
            first = false;
            out.push(WheelStep { delta: chunk * sign, wait_ms: wait });
            sent += chunk;
        }
    }
    out
}

// --------------------------------------------------------------- relay state

/// Humanizing state for one CDP relay session (UNI-293): the preset's
/// config, an rng, and the pointer's last known position (so a click
/// without a preceding `mouseMoved` still gets a path, and a path never
/// starts from nowhere). One per gate connection/session — see
/// [`super::gate`], which turns these plans into CDP commands.
pub struct Humanizer {
    pub cfg: HumanConfig,
    rng: StdRng,
    pointer: (f64, f64),
}

impl Humanizer {
    pub fn new(cfg: HumanConfig) -> Humanizer {
        Humanizer::with_rng(cfg, StdRng::from_os_rng())
    }

    pub fn with_rng(cfg: HumanConfig, mut rng: StdRng) -> Humanizer {
        let pointer = (rand_range(&mut rng, cfg.initial_cursor_x), rand_range(&mut rng, cfg.initial_cursor_y));
        Humanizer { cfg, rng, pointer }
    }

    pub fn pointer(&self) -> (f64, f64) {
        self.pointer
    }

    /// Moves the tracked pointer without generating a path (e.g. a release
    /// at the point already pressed).
    pub fn set_pointer(&mut self, to: (f64, f64)) {
        self.pointer = to;
    }

    /// A path from the tracked pointer to `to`; updates the pointer to `to`.
    pub fn move_to(&mut self, to: (f64, f64)) -> Vec<MoveStep> {
        let path = mouse_path(self.pointer, to, &self.cfg, &mut self.rng);
        self.pointer = to;
        path
    }

    pub fn typing(&mut self, text: &str) -> Vec<KeyStep> {
        typing_plan(text, &self.cfg, &mut self.rng)
    }

    pub fn wheel(&mut self, delta: f64) -> Vec<WheelStep> {
        wheel_plan(delta, &self.cfg, &mut self.rng)
    }

    pub fn rand_range(&mut self, range: Range) -> f64 {
        rand_range(&mut self.rng, range)
    }
}

/// Sometimes, when scrolling to an element, a person scrolls past it and
/// back (the wrapper's scroll overshoot): extra steps to append, or none.
pub fn overshoot_plan<R: Rng + ?Sized>(direction: f64, cfg: &HumanConfig, rng: &mut R) -> Vec<WheelStep> {
    if !rng.random_bool(cfg.scroll_overshoot_chance.clamp(0.0, 1.0)) {
        return Vec::new();
    }
    let over = rand_range(rng, cfg.scroll_overshoot_px).round() * direction.signum();
    let mut out = wheel_plan(over, cfg, rng);
    let mut back = 0.0;
    for k in 0..rand_int(rng, (1, 2)) {
        let corr = if k == 0 { rng.random_range(40.0..=80.0_f64).round() } else { (over.abs() - back).max(10.0) };
        back += corr;
        let mut steps = wheel_plan(-corr * direction.signum(), cfg, rng);
        if let Some(first) = steps.first_mut() {
            first.wait_ms = rand_range(rng, cfg.scroll_settle_delay);
        }
        out.extend(steps);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    fn seeded(n: u64) -> StdRng {
        StdRng::seed_from_u64(n)
    }

    #[test]
    fn bezier_path_starts_near_and_ends_exactly_at_the_target() {
        let cfg = HumanConfig::default_preset();
        for seed in 0..300 {
            let mut rng = seeded(seed);
            let from = (100.0, 80.0);
            let to = (612.0, 431.0);
            let path = mouse_path(from, to, &cfg, &mut rng);
            assert!(path.len() >= cfg.mouse_min_steps as usize, "{} steps", path.len());
            let last = path.last().unwrap();
            assert_eq!((last.x, last.y), to, "ends exactly at the target");
            let first = path[0];
            let step = (to.0 - from.0).hypot(to.1 - from.1) / cfg.mouse_min_steps as f64;
            assert!((first.x - from.0).hypot(first.y - from.1) < step + 3.0, "first step leaves from the start: {first:?}");
        }
    }

    #[test]
    fn bezier_path_time_is_monotone_and_human_paced() {
        let cfg = HumanConfig::default_preset();
        let mut rng = seeded(7);
        let path = mouse_path((0.0, 0.0), (800.0, 300.0), &cfg, &mut rng);
        assert!(path.iter().all(|s| s.wait_ms >= 0.0 && s.wait_ms.is_finite()), "never goes back in time");
        let total: f64 = path.iter().map(|s| s.wait_ms).sum();
        assert!(total > 150.0 && total < 2000.0, "a ~850 px move takes a human time, not 0 ms: {total}");
        assert!(path.iter().filter(|s| s.wait_ms >= cfg.mouse_burst_pause.0).count() >= 5, "moves in bursts");
    }

    #[test]
    fn bezier_path_overshoot_and_wobble_are_bounded() {
        let cfg = HumanConfig::default_preset();
        let (from, to) = ((50.0, 400.0), (450.0, 100.0));
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let dist = f64::hypot(dx, dy);
        let mut overshoots = 0;
        for seed in 0..500 {
            let mut rng = seeded(seed);
            let path = mouse_path(from, to, &cfg, &mut rng);
            for s in &path {
                let (rx, ry) = (s.x - from.0, s.y - from.1);
                let along = (rx * dx + ry * dy) / dist;
                let side = (rx * -dy + ry * dx).abs() / dist;
                // Control points ≤ 30 % of the distance sideways → the curve stays within that, plus wobble/rounding.
                assert!(side <= 0.3 * dist + cfg.mouse_wobble_max + 1.0, "sideways {side}");
                // Never more than the overshoot past the target, never behind the start.
                assert!(along >= -cfg.mouse_wobble_max - 1.0 && along <= dist + cfg.mouse_overshoot_px.1 + 1.0, "along {along}");
            }
            if path.iter().any(|s| (s.x - from.0) * dx + (s.y - from.1) * dy > dist * dist + 1.0) {
                overshoots += 1;
            }
        }
        assert!(overshoots > 20 && overshoots < 150, "overshoots ~15 % of the time: {overshoots}/500");
    }

    #[test]
    fn tiny_moves_are_tiny() {
        let cfg = HumanConfig::default_preset();
        let mut rng = seeded(1);
        assert!(mouse_path((10.0, 10.0), (10.0, 10.0), &cfg, &mut rng).is_empty());
        let path = mouse_path((10.0, 10.0), (14.0, 10.0), &cfg, &mut rng);
        assert!(path.len() <= 6, "no 25 steps for 4 px: {}", path.len());
        assert_eq!(path.last().map(|s| (s.x, s.y)), Some((14.0, 10.0)));
    }

    #[test]
    fn click_target_is_inside_and_off_centre() {
        let cfg = HumanConfig::default_preset();
        let rect = BoxRect { x: 300.0, y: 200.0, width: 160.0, height: 50.0 };
        let mut off_centre = 0;
        for seed in 0..200 {
            let mut rng = seeded(seed);
            let (x, y) = click_target(&rect, false, &cfg, &mut rng);
            assert!(x >= rect.x + rect.width * 0.35 - 1.0 && x <= rect.x + rect.width * 0.65 + 1.0);
            assert!(y >= rect.y + rect.height * 0.35 - 1.0 && y <= rect.y + rect.height * 0.65 + 1.0);
            let (cx, cy) = rect.center();
            if (x - cx).abs() > 3.0 || (y - cy).abs() > 3.0 {
                off_centre += 1;
            }
            let (ix, _) = click_target(&rect, true, &cfg, &mut rng);
            assert!(ix <= rect.x + rect.width * 0.30 + 1.0, "text fields are clicked near the left");
        }
        assert!(off_centre > 150, "rarely within 3 px of the centre: {off_centre}/200");
    }

    /// Replays a typing plan into a text field model.
    fn replay(plan: &[KeyStep]) -> (String, i32) {
        let mut value = String::new();
        let mut shift_depth = 0;
        for step in plan {
            match step {
                KeyStep::Text { text, .. } => value.push_str(text),
                KeyStep::Key { params, .. } => {
                    let key = params["key"].as_str().unwrap();
                    match (params["type"].as_str().unwrap(), key) {
                        ("rawKeyDown", "Shift") => shift_depth += 1,
                        ("keyUp", "Shift") => shift_depth -= 1,
                        ("rawKeyDown", "Backspace") => {
                            value.pop();
                        }
                        ("keyDown", _) => {
                            let text = params["text"].as_str().unwrap();
                            let shifted = params["modifiers"].as_u64() == Some(8);
                            assert_eq!(shifted, shift_depth > 0, "Shift is held exactly for shifted keys ({key})");
                            value.push_str(text);
                        }
                        _ => {}
                    }
                }
            }
        }
        (value, shift_depth)
    }

    #[test]
    fn typing_plan_types_exactly_the_text() {
        let cfg = HumanConfig { mistype_chance: 0.3, ..HumanConfig::default_preset() };
        let text = "Hello, World! jane.doe+x@example.com ~ héllo 日本 😀\nline2\t";
        for seed in 0..50 {
            let plan = typing_plan(text, &cfg, &mut seeded(seed));
            let (typed, shift) = replay(&plan);
            assert_eq!(typed, text, "seed {seed}");
            assert_eq!(shift, 0, "Shift released");
        }
    }

    #[test]
    fn typing_schedule_is_human() {
        let cfg = HumanConfig { mistype_chance: 0.0, typing_pause_chance: 0.0, ..HumanConfig::default_preset() };
        let plan = typing_plan("abcdefghij", &cfg, &mut seeded(3));
        // Each char: keyDown, keyUp held 15–35 ms; 30–110 ms between chars.
        assert_eq!(plan.len(), 20);
        for (i, step) in plan.iter().enumerate() {
            let w = step.wait_ms();
            if i == 0 {
                assert_eq!(w, 0.0);
            } else if i % 2 == 1 {
                assert!((15.0..=35.0).contains(&w), "hold {w}");
            } else {
                assert!((30.0..=110.0).contains(&w), "gap {w}");
            }
        }
        // Capitals and shifted symbols hold Shift; non-ASCII is inserted.
        let plan = typing_plan("A@é", &HumanConfig { mistype_chance: 0.0, ..HumanConfig::default_preset() }, &mut seeded(4));
        let kinds: Vec<String> = plan
            .iter()
            .map(|s| match s {
                KeyStep::Key { params, .. } => format!("{}:{}", params["type"].as_str().unwrap(), params["key"].as_str().unwrap()),
                KeyStep::Text { text, .. } => format!("text:{text}"),
            })
            .collect();
        assert_eq!(
            kinds,
            ["rawKeyDown:Shift", "keyDown:A", "keyUp:A", "keyUp:Shift", "rawKeyDown:Shift", "keyDown:@", "keyUp:@", "keyUp:Shift", "text:é"]
        );
        let KeyStep::Key { params, .. } = &plan[5] else { panic!() };
        assert_eq!((params["code"].as_str(), params["windowsVirtualKeyCode"].as_u64()), (Some("Digit2"), Some(50)));
    }

    #[test]
    fn careful_types_slower() {
        let mean = |cfg: &HumanConfig| {
            let plan = typing_plan(&"x".repeat(200), cfg, &mut seeded(9));
            plan.iter().map(KeyStep::wait_ms).sum::<f64>()
        };
        assert!(mean(&HumanConfig::careful()) > mean(&HumanConfig::default_preset()) * 1.2);
    }

    #[test]
    fn wheel_plan_scrolls_exactly_and_eases() {
        let cfg = HumanConfig::default_preset();
        for (seed, delta) in [(1, 500.0), (2, -300.0), (3, 37.0), (4, 2400.0), (5, -1.0)] {
            let plan = wheel_plan(delta, &cfg, &mut seeded(seed));
            let sum: f64 = plan.iter().map(|s| s.delta).sum();
            assert_eq!(sum, delta, "exact total");
            assert!(plan.iter().all(|s| s.delta != 0.0 && s.delta.signum() == delta.signum() && s.delta.abs() <= 40.0));
            assert!(plan.iter().all(|s| s.wait_ms >= 0.0));
            assert!(plan.iter().all(|s| s.delta.fract() == 0.0), "whole pixels like a wheel");
        }
        let plan = wheel_plan(1000.0, &cfg, &mut seeded(8));
        assert!(plan.len() >= 25, "split in many small events: {}", plan.len());
        assert!(plan.iter().filter(|s| s.wait_ms >= cfg.scroll_pause_fast.0).count() >= 5, "pauses between notches");
        assert!(wheel_plan(0.2, &cfg, &mut seeded(1)).is_empty());
    }

    #[test]
    fn presets_parse() {
        assert_eq!(serde_json::from_str::<Humanize>("\"careful\"").unwrap(), Humanize::Careful);
        assert_eq!(Humanize::default(), Humanize::Default);
        assert!(Humanize::Off.config().is_none());
        assert_eq!(Humanize::Careful.config().unwrap().typing_delay, 100.0);
    }
}
