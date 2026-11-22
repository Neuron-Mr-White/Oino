//! T15: headless phone protocol client. Never uses the local socket for measured ops.
use anyhow::{bail, ensure, Context, Result};
use iroh::{
    endpoint::{presets, Connection},
    Endpoint, EndpointAddr, SecretKey, TransportAddr,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const ALPN: &[u8] = b"unipi/oino/1";
async fn send<W: AsyncWrite + Unpin>(w: &mut W, v: Value) -> Result<usize> {
    let b = unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&v)?);
    w.write_all(&b).await?;
    Ok(b.len())
}
async fn read<R: AsyncRead + Unpin>(r: &mut R) -> Result<(Value, usize)> {
    ensure!(r.read_u8().await? == 0, "expected control frame");
    let n = r.read_u32().await? as usize;
    ensure!(n <= unipi_proto::frame::MAX_FRAME, "oversize frame");
    let mut b = vec![0; n];
    r.read_exact(&mut b).await?;
    Ok((serde_json::from_slice(&b)?, n + 5))
}
fn hello(req: &Value) -> Value {
    json!({"type":"hello","proto":"2.1.0","req":req})
}
fn ok(v: &Value) -> Result<()> {
    ensure!(v["type"] == "ok", "host rejected request: {v}");
    Ok(())
}
async fn local(root: &Path, req: Value) -> Result<Value> {
    let mut s = tokio::net::UnixStream::connect(root.join("app.sock")).await?;
    send(&mut s, hello(&req)).await?;
    let (v, _) = read(&mut s).await?;
    ok(&v)?;
    Ok(v["data"].clone())
}
fn path(c: &Connection) -> Value {
    let paths = c.paths();
    paths.iter().find(iroh::endpoint::Path::is_selected).map_or(Value::Null, |p| json!({"remote":format!("{:?}", p.remote_addr()),"rtt_ms":p.rtt().as_secs_f64()*1000.0}))
}
async fn rpc(connection: &Connection, req: Value) -> Result<(Value, Value)> {
    let started = Instant::now();
    let before = connection.stats();
    let (mut sender, mut receiver) = connection.open_bi().await?;
    let tx = send(&mut sender, hello(&req)).await?;
    sender.finish()?;
    let (reply, rx) = read(&mut receiver).await?;
    let after = connection.stats();
    Ok((
        reply,
        json!({"ms":started.elapsed().as_secs_f64()*1000.0,"frame_tx":tx,"frame_rx":rx,
        "quic_tx":after.udp_tx.bytes.saturating_sub(before.udp_tx.bytes),
        "quic_rx":after.udp_rx.bytes.saturating_sub(before.udp_rx.bytes),"path":path(connection)}),
    ))
}
async fn bind(key: SecretKey, mode: &str) -> Result<Endpoint> {
    let mut b = Endpoint::builder(presets::N0).secret_key(key);
    if mode == "relay" {
        b = b.clear_ip_transports();
    } else {
        b = b
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup();
    }
    Ok(b.bind().await?)
}
async fn connect(ep: &Endpoint, addr: EndpointAddr) -> Result<Connection> {
    Ok(tokio::time::timeout(Duration::from_secs(12), ep.connect(addr, ALPN)).await??)
}
// One rootless UDP forwarder. Each packet is delayed both ways; deterministic loss.
async fn proxy(
    target: std::net::SocketAddr,
    delay: u64,
    loss_every: u64,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let sock = std::sync::Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:0").await?);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], sock.local_addr()?.port()));
    let task = tokio::spawn(async move {
        let mut buf = vec![0; 65536];
        let mut client = None;
        let mut count = 0;
        while let Ok((n, src)) = sock.recv_from(&mut buf).await {
            let dest = if src == target {
                match client {
                    Some(c) => c,
                    None => continue,
                }
            } else {
                client = Some(src);
                target
            };
            count += 1;
            if loss_every > 0 && count % loss_every == 0 {
                continue;
            }
            let data = buf[..n].to_vec();
            let sock = sock.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let _ = sock.send_to(&data, dest).await;
            });
        }
    });
    Ok((addr, task))
}
async fn measure_session(connection: &Connection, session: &str) -> Result<(Value, Value)> {
    let before = connection.stats();
    let started = Instant::now();
    let (mut sender, mut receiver) = connection.open_bi().await?;
    let tx = send(
        &mut sender,
        hello(&json!({"op":"session_open","session":session})),
    )
    .await?;
    let (snapshot, rx) = read(&mut receiver).await?;
    ok(&snapshot)?;
    let after = connection.stats();
    let open = json!({"ms":started.elapsed().as_secs_f64()*1000.0,"frame_tx":tx,"frame_rx":rx,
        "quic_tx":after.udp_tx.bytes.saturating_sub(before.udp_tx.bytes),"quic_rx":after.udp_rx.bytes.saturating_sub(before.udp_rx.bytes),
        "path":path(connection),"page":snapshot["data"]["page"],"entries":snapshot["data"]["entries"].as_array().map(Vec::len)});
    // Set model and wait for its correlated reply (also waits for a record-only wake).
    send(&mut sender, json!({"type":"session_cmd","id":"model","cmd":{"type":"set_model","provider":"mock","modelId":"mock-1"}})).await?;
    loop {
        let (reply, _) = read(&mut receiver).await?;
        if reply["id"] == "model" {
            ok(&reply)?;
            break;
        }
    }
    let before = connection.stats();
    let started = Instant::now();
    let tx = send(&mut sender, json!({"type":"session_cmd","id":"prompt","cmd":{"type":"prompt","message":"Reply with one short sentence. Do not call tools."}})).await?;
    let mut ack = None;
    let mut token = None;
    let mut rx = 0;
    while ack.is_none() || token.is_none() {
        let (reply, frame_bytes) = read(&mut receiver).await?;
        rx += frame_bytes;
        if reply["id"] == "prompt" {
            ok(&reply)?;
            ack = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        if token.is_none() && reply.to_string().contains("text_delta") {
            token = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let prompt_path = path(connection);
    let after = connection.stats();
    let prompt = json!({"path":prompt_path,"ack_ms":ack,"first_token_ms":token,"frame_tx":tx,"frame_rx_through_token_and_ack":rx,
        "quic_tx":after.udp_tx.bytes.saturating_sub(before.udp_tx.bytes),"quic_rx":after.udp_rx.bytes.saturating_sub(before.udp_rx.bytes)});
    send(
        &mut sender,
        json!({"type":"session_cmd","id":"abort","cmd":{"type":"abort"}}),
    )
    .await?;
    loop {
        let (reply, _) = read(&mut receiver).await?;
        if reply["id"] == "abort" {
            ok(&reply)?;
            break;
        }
    }
    connection.close(0u32.into(), b"T15 warm reconnect");
    Ok((open, prompt))
}

fn validate_paths(
    mode: &str,
    proxy_addr: Option<std::net::SocketAddr>,
    initial_path: &Value,
    open: &Value,
    prompt: &Value,
    selected: &Value,
) -> Result<()> {
    if let Some(proxy) = proxy_addr {
        for observed in [initial_path, &open["path"], &prompt["path"], selected] {
            ensure!(
                observed["remote"]
                    .as_str()
                    .unwrap_or("")
                    .contains(&proxy.to_string()),
                "iroh bypassed impairment proxy: {observed}; do not report as impaired"
            );
        }
    }
    if mode == "relay" {
        ensure!(
            selected["remote"].as_str().unwrap_or("").contains("Relay"),
            "not a relay path: {selected}"
        );
    } else if !selected["remote"].as_str().unwrap_or("").contains("Ip") {
        bail!("not an IP path: {selected}");
    }
    Ok(())
}

async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 5,
        "usage: unipi-remote-bench HOST_DIR SESSION direct|relay|impaired LABEL"
    );
    let root = Path::new(&args[1]);
    let session = &args[2];
    let mode = &args[3];
    let label = &args[4];
    ensure!(
        ["direct", "relay", "impaired"].contains(&mode.as_str()),
        "unknown path"
    );
    let cache: Value = serde_json::from_slice(&std::fs::read(root.join("endpoint.json"))?)?;
    let mut addr: EndpointAddr = serde_json::from_value(cache["addr"].clone())?;
    let mut proxy_task = None;
    let mut proxy_addr = None;
    if mode == "relay" {
        addr.addrs.retain(|a| matches!(a, TransportAddr::Relay(_)));
    } else {
        addr.addrs.retain(|a| matches!(a, TransportAddr::Ip(_)));
        if mode == "impaired" {
            let target = addr
                .addrs
                .iter()
                .find_map(|a| match a {
                    TransportAddr::Ip(sender) if sender.is_ipv4() => Some(*sender),
                    _ => None,
                })
                .context("no IPv4 host address")?;
            let (p, task) = proxy(target, 25, 100).await?;
            addr.addrs = [TransportAddr::Ip(p)].into_iter().collect();
            proxy_task = Some(task);
            proxy_addr = Some(p);
        }
    }
    ensure!(
        !addr.addrs.is_empty(),
        "host has no {mode} address; wait for endpoint.json refresh"
    );
    let pair = local(root, json!({"op":"pair_begin"})).await?;
    let key = SecretKey::generate();
    let cold = Instant::now();
    let ep = bind(key.clone(), mode).await?;
    let connection = connect(&ep, addr.clone()).await?;
    let (paired, pair_metric) = rpc(
        &connection,
        json!({"op":"pair","code":pair["code"],"name":"T15 isolated phone"}),
    )
    .await?;
    ok(&paired)?;
    // host_info is local-only in BOTH versions. Preserve the ACL, measure the
    // rejection explicitly and use authenticated sessions_list as readiness.
    let (info, info_metric) = rpc(&connection, json!({"op":"host_info"})).await?;
    ensure!(
        info["type"] == "err" && info["error"]["code"] == "E_UNAUTHORIZED",
        "host_info ACL changed: {info}"
    );
    let (list, list_metric) = rpc(&connection, json!({"op":"sessions_list"})).await?;
    ok(&list)?;
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let initial_path = path(&connection);
    let (open, prompt) = measure_session(&connection, session).await?;
    let started = Instant::now();
    let warm = connect(&ep, addr.clone()).await?;
    let (reply, _) = rpc(&warm, json!({"op":"sessions_list"})).await?;
    ok(&reply)?;
    let warm_ms = started.elapsed().as_secs_f64() * 1000.0;
    warm.close(0u32.into(), b"T15 network change");
    ep.close().await;
    let started = Instant::now();
    let ep = bind(key, mode).await?;
    let rebound = connect(&ep, addr).await?;
    let (reply, _) = rpc(&rebound, json!({"op":"sessions_list"})).await?;
    ok(&reply)?;
    let rebind_ms = started.elapsed().as_secs_f64() * 1000.0;
    let selected = path(&rebound);
    validate_paths(mode, proxy_addr, &initial_path, &open, &prompt, &selected)?;
    println!(
        "{}",
        json!({"label":label,"mode":mode,"cold_ready_ms":cold_ms,"host_info":"local-only; E_UNAUTHORIZED verified",
        "host_info_rejection":info_metric,"pair":pair_metric,"list":list_metric,"open":open,"prompt":prompt,
        "warm_reconnect_ms":warm_ms,"rebind_ms":rebind_ms,"selected_path":selected,"initial_path":initial_path})
    );
    rebound.close(0u32.into(), b"done");
    ep.close().await;
    if let Some(task) = proxy_task {
        task.abort();
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(180), run()).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn proxy_forwards_both_ways_and_drops_deterministically() {
        let server = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let phone = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (addr, task) = proxy(server.local_addr().unwrap(), 5, 3).await.unwrap();
        let start = Instant::now();
        phone.send_to(b"request", addr).await.unwrap();
        let mut buf = [0; 64];
        let (n, src) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"request");
        server.send_to(b"response", src).await.unwrap();
        let (n, _) = phone.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"response");
        assert!(start.elapsed() >= Duration::from_millis(10));
        phone.send_to(b"lost", addr).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), server.recv_from(&mut buf))
                .await
                .is_err()
        );
        task.abort();
    }
    #[tokio::test]
    async fn framing_round_trip_and_oversize() {
        let (mut w, mut r) = tokio::io::duplex(4096);
        let value = hello(&json!({"op":"sessions_list"}));
        let n = send(&mut w, value.clone()).await.unwrap();
        let (got, count) = read(&mut r).await.unwrap();
        assert_eq!(got, value);
        assert_eq!(n, count);
        w.write_all(&[0, 255, 255, 255, 255]).await.unwrap();
        assert!(read(&mut r).await.is_err());
    }
}
