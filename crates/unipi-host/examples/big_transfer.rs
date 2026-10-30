//! UNI-278: big-file download (blob_get) and upload (blob_put) over real iroh,
//! like the phone app does, against a running host — to find where large
//! transfers break. Pairs first with a link from `unipi-host pair`.
//!
//! cargo run --release -p unipi-host --example big_transfer -- --link 'unipi://pair?…' --get /tmp/big300.bin --put /tmp/big60.bin
use std::time::{Duration, Instant};

use iroh::{endpoint::presets, Endpoint, EndpointAddr};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
use unipi_host::{endpoint::ALPN, protocol::read_frame};
use unipi_proto::{frame::{encode, FrameKind}, Control, PROTO_VERSION};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(FrameKind::Control, &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req})).unwrap())
}

async fn control<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<Control> {
    let (k, b) = tokio::time::timeout(Duration::from_secs(60), read_frame(r)).await??;
    anyhow::ensure!(k == 0, "expected control, got kind {k}");
    Ok(serde_json::from_slice(&b)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let link = arg("--link").ok_or_else(|| anyhow::anyhow!("--link required"))?;
    let url = url::Url::parse(&link)?;
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let ticket: iroh_tickets::endpoint::EndpointTicket = q["t"].parse()?;
    let addr: EndpointAddr = ticket.into();
    let endpoint = Endpoint::builder(presets::N0).bind().await?;
    let conn = endpoint.connect(addr, ALPN).await?;
    {
        let (mut s, mut r) = conn.open_bi().await?;
        s.write_all(&hello(serde_json::json!({"op":"pair","code":q["c"],"name":"big-transfer test"}))).await?;
        s.finish()?;
        println!("pair: {:?}", control(&mut r).await?);
    }
    if let Some(path) = arg("--get") {
        let t0 = Instant::now();
        let (mut s, mut r) = conn.open_bi().await?;
        s.write_all(&hello(serde_json::json!({"op":"blob_get","path":path}))).await?;
        s.finish()?;
        let first = control(&mut r).await?;
        println!("blob_get reply: {first:?}");
        let mut got = 0u64;
        let mut h = Sha256::new();
        let mut last = Instant::now();
        loop {
            let (k, b) = match tokio::time::timeout(Duration::from_secs(60), read_frame(&mut r)).await {
                Ok(Ok(f)) => f,
                Ok(Err(e)) => { println!("GET read error after {got} bytes: {e}"); break; }
                Err(_) => { println!("GET stalled 60 s after {got} bytes"); break; }
            };
            if k == 1 {
                got += b.len() as u64;
                h.update(&b);
                if last.elapsed() > Duration::from_secs(2) {
                    println!("  … {} MB ({:.1} MB/s)", got / 1_000_000, got as f64 / 1e6 / t0.elapsed().as_secs_f64());
                    last = Instant::now();
                }
            } else {
                println!("GET end: {:?}", serde_json::from_slice::<Control>(&b)?);
                break;
            }
        }
        println!("GET {got} bytes in {:.1}s sha256 {}", t0.elapsed().as_secs_f64(), hex(&h.finalize()));
    }
    if let Some(path) = arg("--put") {
        let bytes = tokio::fs::read(&path).await?;
        let sha = hex(&Sha256::digest(&bytes));
        let t0 = Instant::now();
        let (mut s, mut r) = conn.open_bi().await?;
        s.write_all(&hello(serde_json::json!({"op":"blob_put","sha256":sha,"size":bytes.len(),"mime":"application/octet-stream","name":"big.bin"}))).await?;
        let reply = control(&mut r).await?;
        println!("blob_put reply: {reply:?}");
        if matches!(reply, Control::Ok { .. }) {
            for (i, piece) in bytes.chunks(256 * 1024).enumerate() {
                if let Err(e) = s.write_all(&encode(FrameKind::Input, piece)).await {
                    println!("PUT write error at chunk {i}: {e}");
                    break;
                }
            }
            s.finish()?;
            println!("PUT end: {:?} in {:.1}s", control(&mut r).await, t0.elapsed().as_secs_f64());
        }
    }
    endpoint.close().await;
    Ok(())
}

