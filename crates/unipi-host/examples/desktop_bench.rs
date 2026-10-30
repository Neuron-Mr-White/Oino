//! Encoder bench for text-heavy screens (UNI-271): encodes a BGRA picture
//! (raw file, `--size WxH`) at a few sizes/bitrates with unipi-host's own
//! encoder, decodes it with openh264 and prints size, encode time and the
//! luma PSNR against the scaled source (what the viewer can at best show).
//!
//! ```text
//! cargo run --release -p unipi-host --example desktop_bench -- \
//!     --input /tmp/text.bgra --size 3440x1440 [--frames 10] [--save /tmp/out]
//! ```

use std::time::Instant;

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use unipi_host::desktop::encode::{bgra_to_i420, encoded_size, Frame, Tuning, H264};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

fn main() -> anyhow::Result<()> {
    let input = arg("--input").expect("--input");
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))).expect("--size WxH");
    let frames: u32 = arg("--frames").and_then(|f| f.parse().ok()).unwrap_or(10);
    let save = arg("--save");
    let bgra = std::fs::read(input)?;
    let src = Frame::new(w, h, bgra);
    // Shifted copy: "scrolled by 3 px" to exercise P frames with motion.
    let mut moved = src.bgra.clone();
    let row = (w * 4) as usize;
    moved.copy_within(3 * row.., 0);
    let moved = Frame::new(w, h, moved);
    let sides: Vec<u32> = arg("--sides").map(|s| s.split(',').map(|v| v.parse().unwrap()).collect()).unwrap_or(vec![1440, 1920, 2560, 3440]);
    let rates: Vec<u32> = arg("--kbps").map(|s| s.split(',').map(|v| v.parse().unwrap()).collect()).unwrap_or(vec![1800, 6000, 12000]);
    for &side in &sides {
        for &kbps in &rates {
            let (ow, oh) = encoded_size(w, h, side, None);
            let mut reference = Vec::new();
            bgra_to_i420(&src, ow, oh, &mut reference);
            let mut tuning = Tuning::default();
            if let Some(q) = arg("--max-qp") { tuning.max_qp = q.parse().unwrap(); }
            if let Some(q) = arg("--min-qp") { tuning.min_qp = q.parse().unwrap(); }
            if let Some(b) = arg("--slice") { tuning.slice_bytes = b.parse().ok().filter(|v: &u32| *v > 0); }
            let mut enc = H264::with_tuning(tuning);
            enc.set_rate(kbps, 30.0);
            let mut dec = Decoder::new()?;
            let mut skipped = 0u32;
            let (mut total_ms, mut key_bytes, mut delta_bytes, mut n) = (0.0f64, 0usize, 0usize, 0u32);
            let mut key_psnr = 0.0;
            let mut last_psnr = 0.0;
            let tc = Instant::now();
            let mut tmp = Vec::new();
            bgra_to_i420(&src, ow, oh, &mut tmp);
            let convert_ms = tc.elapsed().as_secs_f64() * 1000.0;
            for t in 0..frames {
                let frame = if t % 2 == 1 { &moved } else { &src };
                let t0 = Instant::now();
                let Some(out) = enc.encode(frame, ow, oh).map_err(anyhow::Error::msg)? else { skipped += 1; continue };
                total_ms += t0.elapsed().as_secs_f64() * 1000.0;
                n += 1;
                if out.key { key_bytes = out.data.len() } else { delta_bytes += out.data.len() }
                if let Some(yuv) = dec.decode(&out.data)? {
                    if t % 2 == 0 {
                        let (yw, yh) = yuv.dimensions();
                        let mut y = Vec::with_capacity(yw * yh);
                        let (ys, _, _) = yuv.strides();
                        for r in 0..yh {
                            y.extend_from_slice(&yuv.y()[r * ys..r * ys + yw]);
                        }
                        let p = psnr(&y, &reference[..yw * yh]);
                        if out.key { key_psnr = p }
                        last_psnr = p;
                        if let (Some(dir), true) = (&save, t + 2 >= frames) {
                            let mut rgba = vec![0u8; yw * yh * 4];
                            yuv.write_rgba8(&mut rgba);
                            let mut ppm = format!("P6 {yw} {yh} 255\n").into_bytes();
                            for px in rgba.chunks_exact(4) {
                                ppm.extend_from_slice(&px[..3]);
                            }
                            std::fs::create_dir_all(dir)?;
                            std::fs::write(format!("{dir}/bench-{side}-{kbps}.ppm"), ppm)?;
                        }
                    }
                }
            }
            // Idle: identical frames, then a forced key frame (the rate
            // control has saved budget) — how sharp is the "idle refresh"?
            let mut idle_key = (0usize, 0.0f64);
            for t in 0..20 {
                if t == 19 { enc.request_key(); }
                if let Some(out) = enc.encode(&src, ow, oh).map_err(anyhow::Error::msg)? {
                    if let Some(yuv) = dec.decode(&out.data)? {
                        if std::env::var("BENCH_IDLE").is_ok() && t < 6 {
                            let (yw, yh) = yuv.dimensions();
                            let (ys, _, _) = yuv.strides();
                            let mut y = Vec::new();
                            for r in 0..yh { y.extend_from_slice(&yuv.y()[r * ys..r * ys + yw]); }
                            println!("    idle t={t} {} B psnr {:.1}", out.data.len(), psnr(&y, &reference[..yw * yh]));
                        }
                        if out.key && t == 19 {
                            let (yw, yh) = yuv.dimensions();
                            let (ys, _, _) = yuv.strides();
                            let mut y = Vec::new();
                            for r in 0..yh { y.extend_from_slice(&yuv.y()[r * ys..r * ys + yw]); }
                            idle_key = (out.data.len() / 1024, psnr(&y, &reference[..yw * yh]));
                        }
                    }
                }
            }
            println!("    idle key {} KB psnr {:.1} dB", idle_key.0, idle_key.1);
            println!(
                "{ow}x{oh} {kbps}kbps: key {} KB psnr {key_psnr:.1} dB · after motion psnr {last_psnr:.1} dB · delta avg {} KB · encode avg {:.1} ms (convert {convert_ms:.1}) · skipped {skipped}/{frames}",
                key_bytes / 1024,
                delta_bytes / 1024 / (n.saturating_sub(1).max(1) as usize),
                total_ms / n.max(1) as f64
            );
        }
    }
    Ok(())
}
