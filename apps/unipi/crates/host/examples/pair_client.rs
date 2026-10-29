//! A test phone for pairing checks (UNI-266): pairs with a host from its
//! `unipi://pair?t=…&c=…&n=…` link (what the QR holds), as the app does.
//!
//! ```text
//! cargo run -p unipi-host --example pair_client -- --link 'unipi://pair?…' [--name 'Pixel 9']
//! ```
//! Prints `paired with <host> (<host id>)`; exit 1 when the host refuses.

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use unipi_host::endpoint::ALPN;
use unipi_host::protocol::read_frame;
use unipi_proto::Control;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let link = arg("--link").ok_or_else(|| anyhow::anyhow!("--link 'unipi://pair?…' is required"))?;
    let name = arg("--name").unwrap_or_else(|| "pair-client".into());
    let url = url::Url::parse(&link)?;
    anyhow::ensure!(url.scheme() == "unipi" && url.host_str() == Some("pair"), "not a unipi://pair link");
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    let ticket: iroh_tickets::endpoint::EndpointTicket = q.get("t").ok_or_else(|| anyhow::anyhow!("no ticket"))?.parse()?;
    let addr: EndpointAddr = ticket.into();
    let endpoint = Endpoint::builder(presets::N0).bind().await?;
    let conn = endpoint.connect(addr, ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let hello = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":{"op":"pair","code":q.get("c").cloned().unwrap_or_default(),"name":name}});
    send.write_all(&unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&hello)?)).await?;
    send.finish()?;
    let (_, bytes) = read_frame(&mut recv).await?;
    match serde_json::from_slice::<Control>(&bytes)? {
        Control::Ok { data } => {
            let data = data.unwrap_or_default();
            println!("paired with {} ({})", data["host_name"].as_str().unwrap_or("?"), data["host_id"].as_str().unwrap_or("?"));
            conn.close(0u32.into(), b"done");
            endpoint.close().await;
            Ok(())
        }
        other => {
            eprintln!("refused: {other:?}");
            std::process::exit(1);
        }
    }
}
