//! Captured frames → H.264 (openh264, built from source so it also links
//! into the static musl release). Screen-content tuning, adaptive bitrate
//! and fps, key frames on request.

use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, QpRange, RateControlMode, SpsPpsStrategy, UsageType,
};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;
use unipi_proto::DesktopQuality;

/// One captured frame, tightly packed BGRA/BGRx (what PipeWire, X11 and
/// most compositors hand out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width * 4` bytes per row, `height` rows.
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn new(width: u32, height: u32, bgra: Vec<u8>) -> Frame {
        debug_assert_eq!(bgra.len(), (width * height * 4) as usize);
        Frame { width, height, bgra }
    }

    /// Repacks rows with padding (`stride` bytes each) into a tight frame.
    pub fn from_strided(width: u32, height: u32, stride: usize, data: &[u8]) -> Option<Frame> {
        let row = width as usize * 4;
        if stride < row || data.len() < stride * (height as usize - 1) + row {
            return None;
        }
        let mut bgra = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            bgra.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        Some(Frame { width, height, bgra })
    }
}

/// Encoding targets for one rung of a quality preset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// Longest side of the encoded picture (capped at the screen: never upscaled).
    pub max_side: u32,
    pub fps: f32,
    pub kbps: u32,
    /// QP ceiling: the rate control drops frames rather than smearing text.
    pub max_qp: u8,
    /// Also cap the size at the viewer's own longest side (device px).
    pub viewport_cap: bool,
}

const fn t(max_side: u32, fps: f32, kbps: u32, max_qp: u8, viewport_cap: bool) -> Target {
    Target { max_side, fps, kbps, max_qp, viewport_cap }
}

/// "Native": the screen's own size (openh264 tops out at 3840).
pub const NATIVE: u32 = 3840;

/// UNI-271 presets (RustDesk's Best quality / Balanced / Optimize reaction
/// time). Each is a ladder the session walks with the link: down at once when
/// the viewer falls behind, up after calm seconds. `best` keeps the screen's
/// own resolution and gives up fps and bitrate first (text stays sharp);
/// `balanced` caps at 2560 px; `speed` follows the viewer's size.
pub const BEST_LADDER: [Target; 5] = [
    t(NATIVE, 30.0, 16000, 30, false),
    t(NATIVE, 20.0, 10000, 32, false),
    t(NATIVE, 12.0, 6000, 34, false),
    t(2560, 12.0, 4000, 34, false),
    t(1920, 10.0, 2500, 36, false),
];
pub const BALANCED_LADDER: [Target; 5] = [
    t(2560, 30.0, 8000, 34, false),
    t(2560, 20.0, 5000, 36, false),
    t(1920, 20.0, 3000, 38, false),
    t(1600, 15.0, 1800, 40, false),
    t(1280, 10.0, 900, 42, false),
];
pub const SPEED_LADDER: [Target; 4] = [
    t(1600, 30.0, 3000, 40, true),
    t(1280, 30.0, 1800, 42, true),
    t(1280, 20.0, 1000, 44, true),
    t(960, 15.0, 500, 46, true),
];
/// The pre-1.13 `auto` ladder, low → high (older apps still send `auto`).
/// UNI-271: starts at 1920 px and the viewport cap is at least 1920 px (it
/// used to start at 1600 and cap at the phone's width: a 3440 screen became
/// 1080 px wide on a phone and 1440 px in a desktop window).
pub const AUTO_LADDER: [Target; 5] = [
    t(960, 10.0, 400, 51, true),
    t(1280, 15.0, 900, 51, true),
    t(1600, 20.0, 1800, 51, true),
    t(1920, 30.0, 3500, 51, true),
    t(2560, 30.0, 6000, 51, true),
];
/// Where `auto` starts (index into the low → high `AUTO_LADDER`).
pub const AUTO_START: u8 = 3;
/// The legacy `auto` viewport cap never goes below this.
pub const AUTO_MIN_CAP: u32 = 1920;

const LOW: [Target; 1] = [t(1280, 15.0, 800, 51, true)];
const MEDIUM: [Target; 1] = [t(1600, 24.0, 2500, 51, true)];
const HIGH: [Target; 1] = [t(2560, 30.0, 6000, 51, true)];

/// Rungs of a preset's ladder: level 0 is the best picture, higher levels
/// are what the session falls back to (fixed presets have one rung).
pub fn ladder(quality: DesktopQuality) -> &'static [Target] {
    match quality {
        DesktopQuality::Best => &BEST_LADDER,
        DesktopQuality::Balanced => &BALANCED_LADDER,
        DesktopQuality::Speed => &SPEED_LADDER,
        DesktopQuality::Low => &LOW,
        DesktopQuality::Medium => &MEDIUM,
        DesktopQuality::High => &HIGH,
        DesktopQuality::Auto => &AUTO_LADDER,
    }
}

/// Level the session starts at. Level numbers grow with *worse* quality,
/// except for the legacy `auto` ladder (low → high, see `adapt`).
pub fn start_level(quality: DesktopQuality) -> u8 {
    if quality == DesktopQuality::Auto { AUTO_START } else { 0 }
}

/// Does a better picture mean a lower level? (Everything but legacy `auto`.)
pub fn best_first(quality: DesktopQuality) -> bool {
    quality != DesktopQuality::Auto
}

pub fn target_for(quality: DesktopQuality, level: u8) -> Target {
    let rungs = ladder(quality);
    rungs[(level as usize).min(rungs.len() - 1)]
}

/// The viewport cap a target applies (device px), if any.
pub fn viewport_limit(quality: DesktopQuality, target: &Target, viewport_max: Option<u32>) -> Option<u32> {
    if !target.viewport_cap {
        return None;
    }
    viewport_max.map(|v| if quality == DesktopQuality::Auto { v.max(AUTO_MIN_CAP) } else { v })
}

/// Quiet time before the session re-sends the picture as a sharp key frame.
pub const IDLE_SHARPEN_MS: u64 = 1200;
/// A delta frame bigger than this means "something big moved" (scrolling).
pub const IDLE_DELTA_BYTES: usize = 12000;
/// "Idle" = the small deltas of the quiet window use at most this share of
/// the rung's bitrate (a clock or a caret: ~30 KB/s ≈ 2 % of 16 Mbps).
pub const IDLE_RATE_SHARE: f64 = 0.1;
/// While sharp, a delta bigger than this (or a skipped frame) goes back to
/// the normal tuning.
pub const SHARP_EXIT_BYTES: usize = 40000;

/// QP ceiling for the "sharpen on idle" key frame of a preset (None: no sharpening).
pub fn sharp_qp(quality: DesktopQuality) -> Option<u8> {
    match quality {
        DesktopQuality::Best => Some(22),
        DesktopQuality::Balanced => Some(26),
        _ => None,
    }
}

/// The encoded size for a `w×h` screen: longest side ≤ `max_side` (and ≤
/// the phone's own longest side when given), never upscaled, even numbers
/// (4:2:0), at most 3840×2160 (openh264's level limit).
pub fn encoded_size(width: u32, height: u32, max_side: u32, viewport_max: Option<u32>) -> (u32, u32) {
    let mut limit = max_side.min(3840);
    if let Some(v) = viewport_max {
        limit = limit.min(v.max(320));
    }
    let longest = width.max(height).max(1);
    let scale = (limit as f64 / longest as f64).min(1.0);
    let even = |v: f64| ((v.round() as u32).max(2) / 2) * 2;
    let (mut w, mut h) = (even(width as f64 * scale), even(height as f64 * scale));
    if w.min(h) > 2160 {
        let s = 2160.0 / w.min(h) as f64;
        w = even(w as f64 * s);
        h = even(h as f64 * s);
    }
    (w, h)
}

/// BGRA (any size) → I420 at `ow×oh` (box-filtered when shrinking), BT.601
/// limited range (the H.264 default the decoders assume). Row bands run on
/// several threads (a 3440×1440 frame: ~20 ms on one core).
pub fn bgra_to_i420(frame: &Frame, ow: u32, oh: u32, out: &mut Vec<u8>) {
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let (ow, oh) = (ow as usize, oh as usize);
    let y_len = ow * oh;
    let c_len = (ow / 2) * (oh / 2);
    out.clear();
    out.resize(y_len + 2 * c_len, 0);
    // For every output pixel: average a box of source pixels.
    let xmap: Vec<(usize, usize)> = (0..ow).map(|x| (x * sw / ow, ((x + 1) * sw / ow).max(x * sw / ow + 1).min(sw))).collect();
    let ymap: Vec<(usize, usize)> = (0..oh).map(|y| (y * sh / oh, ((y + 1) * sh / oh).max(y * sh / oh + 1).min(sh))).collect();
    let (yp, rest) = out.split_at_mut(y_len);
    let (up, vp) = rest.split_at_mut(c_len);
    let rows = oh / 2;
    let threads = if ow * oh >= 640 * 480 { std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(1) } else { 1 };
    let band = rows.div_ceil(threads.max(1)).max(1);
    let cw = ow / 2;
    let ctx = Ctx { src: &frame.bgra, sw, ow, xmap: &xmap, ymap: &ymap };
    std::thread::scope(|scope| {
        let mut ys = yp.chunks_mut(band * 2 * ow);
        let mut us = up.chunks_mut((band * cw).max(1));
        let mut vs = vp.chunks_mut((band * cw).max(1));
        let mut first = 0;
        while first < rows {
            let (Some(y), Some(u), Some(v)) = (ys.next(), us.next(), vs.next()) else { break };
            let start = first;
            let ctx = &ctx;
            if threads == 1 {
                convert_band(ctx, start, y, u, v);
            } else {
                scope.spawn(move || convert_band(ctx, start, y, u, v));
            }
            first += band;
        }
    });
}

struct Ctx<'a> {
    src: &'a [u8],
    sw: usize,
    ow: usize,
    xmap: &'a [(usize, usize)],
    ymap: &'a [(usize, usize)],
}

/// Converts chroma rows `first..` (as many as `u` holds) and their luma pairs.
fn convert_band(ctx: &Ctx, first: usize, yp: &mut [u8], up: &mut [u8], vp: &mut [u8]) {
    let ow = ctx.ow;
    let cw = ow / 2;
    let rows = up.len().checked_div(cw).unwrap_or(0);
    let mut rgb_row = vec![(0u32, 0u32, 0u32); ow * 2];
    for local in 0..rows {
        let cy = first + local;
        for dy in 0..2 {
            let y = cy * 2 + dy;
            let (y0, y1) = ctx.ymap[y];
            for (x, &(x0, x1)) in ctx.xmap.iter().enumerate() {
                let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
                // Cap the box at 4×4 samples: plenty for screen content.
                let ystep = ((y1 - y0) / 4).max(1);
                let xstep = ((x1 - x0) / 4).max(1);
                let mut sy = y0;
                while sy < y1 {
                    let row = sy * ctx.sw * 4;
                    let mut sx = x0;
                    while sx < x1 {
                        let i = row + sx * 4;
                        b += ctx.src[i] as u32;
                        g += ctx.src[i + 1] as u32;
                        r += ctx.src[i + 2] as u32;
                        n += 1;
                        sx += xstep;
                    }
                    sy += ystep;
                }
                let (r, g, b) = (r / n, g / n, b / n);
                yp[(local * 2 + dy) * ow + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
                rgb_row[dy * ow + x] = (r, g, b);
            }
        }
        for cx in 0..cw {
            let mut s = (0i32, 0i32, 0i32);
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let (r, g, b) = rgb_row[dy * ow + cx * 2 + dx];
                s.0 += r as i32;
                s.1 += g as i32;
                s.2 += b as i32;
            }
            let (r, g, b) = (s.0 / 4, s.1 / 4, s.2 / 4);
            up[local * cw + cx] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            vp[local * cw + cx] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

/// One encoded access unit (Annex-B, SPS/PPS in front of every IDR).
#[derive(Debug, Clone)]
pub struct Encoded {
    pub data: Vec<u8>,
    pub key: bool,
    pub width: u32,
    pub height: u32,
}

/// Encoder knobs besides rate and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    /// Upper QP bound: keeps text readable; the rate control skips frames
    /// instead of smearing them.
    pub max_qp: u8,
    pub min_qp: u8,
    /// Size-limited slices (bytes). `None` = one slice.
    pub slice_bytes: Option<u32>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { max_qp: 51, min_qp: 10, slice_bytes: None }
    }
}

/// The H.264 encoder with the knobs the session turns.
pub struct H264 {
    tuning: Tuning,
    encoder: Option<Encoder>,
    size: (u32, u32),
    kbps: u32,
    fps: f32,
    force_key: bool,
    yuv: Vec<u8>,
}

impl Default for H264 {
    fn default() -> Self {
        H264::new()
    }
}

impl H264 {
    pub fn new() -> H264 {
        H264::with_tuning(Tuning::default())
    }

    pub fn with_tuning(tuning: Tuning) -> H264 {
        H264 { tuning, encoder: None, size: (0, 0), kbps: 0, fps: 0.0, force_key: true, yuv: Vec::new() }
    }

    fn build(kbps: u32, fps: f32, tuning: Tuning) -> Result<Encoder, String> {
        let threads = std::thread::available_parallelism().map(|n| n.get().min(8) as u16).unwrap_or(1);
        let mut config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(fps))
            .profile(Profile::Baseline)
            .complexity(Complexity::Medium)
            .qp(QpRange::new(tuning.min_qp, tuning.max_qp))
            .sps_pps_strategy(SpsPpsStrategy::ConstantId)
            // A key frame every ~10 s anyway: a phone that lost one recovers.
            .intra_frame_period(IntraFramePeriod::from_num_frames((fps * 10.0) as u32))
            .adaptive_quantization(false)
            .background_detection(false)
            // Skipping is how the rate control keeps the bitrate; the
            // session re-encodes a skipped picture on its next tick.
            .skip_frames(true)
            .num_threads(threads);
        if let Some(bytes) = tuning.slice_bytes {
            config = config.max_slice_len(bytes);
        }
        Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|e| e.to_string())
    }

    /// Asks for an IDR on the next frame.
    pub fn request_key(&mut self) {
        self.force_key = true;
    }

    /// Changes the QP range (rebuilds → key frame when it changed).
    pub fn set_tuning(&mut self, tuning: Tuning) {
        if tuning != self.tuning {
            self.tuning = tuning;
            self.encoder = None;
        }
    }

    pub fn tuning(&self) -> Tuning {
        self.tuning
    }

    /// Changes bitrate/fps; rebuilds the encoder (and so sends a key frame)
    /// only when they really changed.
    pub fn set_rate(&mut self, kbps: u32, fps: f32) {
        if kbps != self.kbps || (fps - self.fps).abs() > 0.5 {
            self.kbps = kbps;
            self.fps = fps;
            self.encoder = None;
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Scales + converts + encodes. `None` when the encoder skipped the frame.
    pub fn encode(&mut self, frame: &Frame, width: u32, height: u32) -> Result<Option<Encoded>, String> {
        if frame.width < 2 || frame.height < 2 {
            return Err("frame too small".into());
        }
        if (width, height) != self.size {
            self.size = (width, height);
            self.encoder = None;
        }
        if self.encoder.is_none() {
            let (kbps, fps) = if self.kbps == 0 { (1800, 20.0) } else { (self.kbps, self.fps) };
            self.kbps = kbps;
            self.fps = fps;
            self.encoder = Some(H264::build(kbps, fps, self.tuning)?);
            self.force_key = true;
        }
        bgra_to_i420(frame, width, height, &mut self.yuv);
        let (w, h) = (width as usize, height as usize);
        let (y, rest) = self.yuv.split_at(w * h);
        let (u, v) = rest.split_at((w / 2) * (h / 2));
        let source = YUVSlices::new((y, u, v), (w, h), (w, w / 2, w / 2));
        let encoder = self.encoder.as_mut().expect("built above");
        if std::mem::take(&mut self.force_key) {
            encoder.force_intra_frame();
        }
        let stream = encoder.encode(&source).map_err(|e| e.to_string())?;
        let key = matches!(stream.frame_type(), openh264::encoder::FrameType::IDR | openh264::encoder::FrameType::I);
        let data = stream.to_vec();
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some(Encoded { data, key, width, height }))
    }
}

/// WebCodecs codec string for what we emit (Constrained Baseline, level
/// from the picture size).
pub fn codec_string(width: u32, height: u32) -> String {
    let mbs = width.div_ceil(16) * height.div_ceil(16);
    let level = match mbs {
        0..=3600 => 0x1f,     // 3.1 (up to 1280×720)
        3601..=8192 => 0x28,  // 4.0 (1080p)
        8193..=22080 => 0x32, // 5.0 (1440p)
        _ => 0x33,            // 5.1 (4K)
    };
    format!("avc1.42e0{level:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::decoder::Decoder;
    use openh264::formats::YUVSource;

    /// A test card: colour bars + a moving white box.
    pub fn card(width: u32, height: u32, t: u32) -> Frame {
        let colors = [[255, 255, 255], [0, 255, 255], [255, 255, 0], [0, 255, 0], [255, 0, 255], [0, 0, 255], [255, 0, 0], [0, 0, 0]];
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let c = colors[(x * 8 / width) as usize];
                let i = ((y * width + x) * 4) as usize;
                bgra[i] = c[2];
                bgra[i + 1] = c[1];
                bgra[i + 2] = c[0];
                bgra[i + 3] = 255;
            }
        }
        let bx = (t * 7) % (width - 40);
        for y in 10..50.min(height) {
            for x in bx..bx + 40 {
                let i = ((y * width + x) * 4) as usize;
                bgra[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        Frame::new(width, height, bgra)
    }

    #[test]
    fn sizes() {
        assert_eq!(encoded_size(2560, 1440, 1280, None), (1280, 720));
        assert_eq!(encoded_size(1280, 720, 2560, None), (1280, 720), "never upscales");
        assert_eq!(encoded_size(1920, 1080, 2560, Some(1000)), (1000, 562));
        assert_eq!(encoded_size(7680, 4320, 9000, None), (3840, 2160));
        assert_eq!(encoded_size(1366, 768, 1366, None), (1366, 768));
        assert_eq!(encoded_size(1365, 767, 4000, None), (1364, 766), "even sides");
    }

    #[test]
    fn strided_repack() {
        let data: Vec<u8> = (0..(3 * 12 + 8) as u8).collect(); // 2px wide, stride 12, 3 rows (last row short padding)
        let f = Frame::from_strided(2, 3, 12, &data).unwrap();
        assert_eq!(f.bgra.len(), 24);
        assert_eq!(&f.bgra[8..16], &data[12..20]);
        assert!(Frame::from_strided(4, 3, 12, &data).is_none());
    }

    #[test]
    fn colour_conversion() {
        let white = Frame::new(4, 4, vec![255; 64]);
        let mut out = Vec::new();
        bgra_to_i420(&white, 4, 4, &mut out);
        assert!(out[..16].iter().all(|&y| y == 235), "{:?}", &out[..16]);
        assert!(out[16..].iter().all(|&c| (127..=129).contains(&c)));
        // Pure red (B,G,R = 0,0,255) → Y≈82, V high.
        let red = Frame::new(2, 2, [0, 0, 255, 255].repeat(4));
        bgra_to_i420(&red, 2, 2, &mut out);
        assert!((80..=84).contains(&out[0]), "{}", out[0]);
        assert!(out[5] > 230, "V {}", out[5]);
        // Downscale 4×4 → 2×2 keeps size.
        bgra_to_i420(&white, 2, 2, &mut out);
        assert_eq!(out.len(), 6);
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut enc = H264::new();
        enc.set_rate(2000, 20.0);
        let mut dec = Decoder::new().unwrap();
        let (w, h) = encoded_size(1280, 720, 640, None);
        let mut keys = 0;
        let mut decoded = 0;
        for t in 0..20 {
            if t == 10 {
                enc.request_key();
            }
            let frame = card(1280, 720, t);
            let Some(out) = enc.encode(&frame, w, h).unwrap() else { continue };
            assert_eq!((out.width, out.height), (640, 360));
            assert_eq!(&out.data[..4], &[0, 0, 0, 1], "Annex-B start code");
            if out.key {
                keys += 1;
                // SPS (NAL 7) leads every key frame.
                assert_eq!(out.data[4] & 0x1f, 7);
            }
            if t == 0 {
                assert!(out.key, "first frame is a key frame");
            }
            if let Some(yuv) = dec.decode(&out.data).unwrap() {
                assert_eq!(yuv.dimensions(), (640, 360));
                let mut rgba = vec![0u8; 640 * 360 * 4];
                yuv.write_rgba8(&mut rgba);
                // Left bar is white, 7th bar is red-ish.
                let px = |x: usize, y: usize| &rgba[(y * 640 + x) * 4..(y * 640 + x) * 4 + 3];
                assert!(px(20, 200).iter().all(|&c| c > 200), "white bar {:?}", px(20, 200));
                let red = px(6 * 80 + 40, 200);
                assert!(red[0] > 180 && red[1] < 70 && red[2] < 70, "red bar {red:?}");
                decoded += 1;
            }
        }
        assert!(keys >= 2, "initial + requested key frame, got {keys}");
        assert!(decoded >= 18, "decoded {decoded}");
    }

    #[test]
    fn rate_change_rebuilds_with_key() {
        let mut enc = H264::new();
        enc.set_rate(1000, 15.0);
        let f = card(320, 240, 0);
        assert!(enc.encode(&f, 320, 240).unwrap().unwrap().key);
        let second = enc.encode(&card(320, 240, 1), 320, 240).unwrap().unwrap();
        assert!(!second.key);
        enc.set_rate(400, 10.0);
        assert!(enc.encode(&card(320, 240, 2), 320, 240).unwrap().unwrap().key);
        // A size change also restarts with a key frame.
        assert!(enc.encode(&card(320, 240, 3), 160, 120).unwrap().unwrap().key);
    }

    #[test]
    fn codec_strings() {
        assert_eq!(codec_string(1280, 720), "avc1.42e01f");
        assert_eq!(codec_string(1920, 1080), "avc1.42e028");
        assert_eq!(codec_string(2560, 1440), "avc1.42e032");
        assert_eq!(codec_string(3840, 2160), "avc1.42e033");
    }
}
