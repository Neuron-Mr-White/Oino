//! iroh endpoint management and the one-bistream-per-op connection
//! model (DESIGN.md §5.2–§5.4).
//!
//! The endpoint is created lazily on first use, persists its
//! [`iroh::SecretKey`] in the app data dir so the phone's endpoint id
//! is stable across restarts, and is shared (one per process) behind
//! a `tokio::sync::OnceCell` so every command reuses the same
//! endpoint and its connection cache.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use iroh::endpoint::{Connection, RelayMode};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayUrl, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use iroh_tickets::Ticket;

use unipi_proto::{AppErrorDto, ProtoError};

use crate::op_stream::{OpRecv, OpSend};
use crate::path_kind::{classify_with_local, PathKind};

/// The ALPN for `unipi/oino/1` (v2: the old unipi app-host speaks
/// `unipi/app/1`, so the two generations never cross; spec host.md §5.1).
pub const ALPN: &[u8] = b"unipi/oino/1";

const SECRET_KEY_FILE: &str = "iroh-secret-key";

/// Per-process iroh state: the bound endpoint and a cache of open
/// connections, keyed by the remote endpoint id (hex), so repeated
/// commands against the same host reuse one connection instead of
/// dialing again for every operation.
pub struct NetState {
    cell: tokio::sync::Mutex<Option<(Option<String>, Endpoint)>>,
    connections: Mutex<HashMap<String, Connection>>,
}

impl Default for NetState {
    fn default() -> Self {
        NetState {
            cell: tokio::sync::Mutex::new(None),
            connections: Mutex::new(HashMap::new()),
        }
    }
}

impl NetState {
    /// Returns the shared endpoint, binding it on first use. `relay`
    /// selects the n0 default relays unless a custom relay URL is
    /// passed (DESIGN.md §3.10 "relay: n0 public relays ... or a
    /// custom relay URL").
    pub async fn endpoint(
        &self,
        app_data_dir: &Path,
        relay: Option<&str>,
    ) -> Result<Endpoint, AppErrorDto> {
        let relay = relay.filter(|value| *value != "n0").map(str::to_owned);
        let mut cell = self.cell.lock().await;
        if let Some((current, endpoint)) = cell.as_ref() {
            if current == &relay {
                return Ok(endpoint.clone());
            }
        }
        let endpoint = bind_endpoint(app_data_dir.to_path_buf(), relay.clone()).await?;
        self.connections
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        if let Some((_, old)) = cell.take() {
            old.close().await;
        }
        *cell = Some((relay, endpoint.clone()));
        Ok(endpoint)
    }

    /// The relay of the currently bound endpoint (`None` = n0 or not yet
    /// bound). Commands that don't carry a relay (the board) reuse it so
    /// they never rebind the shared endpoint.
    pub async fn current_relay(&self) -> Option<String> {
        self.cell
            .lock()
            .await
            .as_ref()
            .and_then(|(relay, _)| relay.clone())
    }

    /// Returns a cached connection to `remote_id` if one is open, or
    /// opens and caches a new one by dialing `ticket`.
    pub async fn connection_for(
        &self,
        app_data_dir: &Path,
        relay: Option<&str>,
        ticket: &str,
    ) -> Result<Connection, AppErrorDto> {
        let parsed = parse_ticket(ticket)?;
        let remote_id = parsed.endpoint_addr().id.to_string();

        let endpoint = self.endpoint(app_data_dir, relay).await?;
        if let Some(conn) = self.cached_live(&remote_id) {
            return Ok(conn);
        }
        let conn = dial(&endpoint, parsed.endpoint_addr().clone()).await?;

        self.connections
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(remote_id, conn.clone());
        Ok(conn)
    }

    /// UNI-230 + v2: one fresh op stream to the host behind `ticket` — the
    /// single entry point every command uses (the transport is chosen here:
    /// this PC's own host over its local transport (`app.sock`, or the named
    /// pipe on Windows — spec host.md §6.1), everything else over iroh).
    pub async fn open_op(
        &self,
        app_data_dir: &Path,
        relay: Option<&str>,
        ticket: &str,
    ) -> Result<(OpSend, OpRecv), AppErrorDto> {
        // UNI-231: this PC's own host → its local transport (offline-proof);
        // if it is gone, fall back to iroh.
        if let Ok(parsed) = parse_ticket(ticket) {
            if let Some(local) = crate::local_host::socket_for(&parsed.endpoint_addr().id.to_string()) {
                match crate::local_host::connect(&local).await {
                    Ok(halves) => return Ok(halves),
                    Err(error) => log::warn!("local host {}: {error}; using iroh", local.socket.display()),
                }
            }
        }
        let conn = self.connection_for(app_data_dir, relay, ticket).await?;
        open_op_stream(&conn).await
    }

    /// The path the op streams to `ticket`'s host take right now (the chip).
    pub fn path_for(&self, ticket: &str) -> PathKind {
        let Ok(parsed) = parse_ticket(ticket) else { return PathKind::Unknown };
        let id = parsed.endpoint_addr().id.to_string();
        if crate::local_host::socket_for(&id).is_some() {
            return PathKind::Local;
        }
        self.cached_live(&id).map(|c| selected_path(&c).0).unwrap_or(PathKind::Unknown)
    }

    pub fn drop_connection(&self, id: &str) {
        if let Some(connection) = self
            .connections
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(id)
        {
            connection.close(0u32.into(), b"reconnect");
        }
    }
    fn cached_live(&self, remote_id: &str) -> Option<Connection> {
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match connections.get(remote_id) {
            Some(conn) if conn.close_reason().is_none() => Some(conn.clone()),
            Some(_) => {
                connections.remove(remote_id);
                None
            }
            None => None,
        }
    }
}

/// Parses an iroh ticket from either the bare ticket string or a
/// `unipi://pair?t=<ticket>&c=<code>` deep-link URL (see
/// `parse_pair_link`). Used by `pair_host` and by `list_sessions`
/// when re-dialing a paired host from its stored ticket.
pub fn parse_ticket(raw: &str) -> Result<EndpointTicket, AppErrorDto> {
    let raw = raw.trim();
    EndpointTicket::decode_string(raw).map_err(|error| {
        ProtoError::PairingInvalid(format!("invalid pairing ticket: {error}")).to_app_error()
    })
}

/// A parsed `unipi://pair` deep link: the encoded ticket and the
/// one-time pairing code.
pub struct PairLink {
    pub ticket: String,
    pub code: String,
}

/// Parses a pairing payload, which is either:
/// - a bare ticket string (`endpoint...`), or
/// - a `unipi://pair?t=<ticket>&c=<code>` deep-link URL.
///
/// The host's QR code (DESIGN.md §3.2) encodes the deep-link form so
/// scanning it both opens the app and carries the one-time code; a
/// bare ticket has no code and pairing then only validates the
/// endpoint ticket (no code to prove).
pub fn parse_pair_link(input: &str) -> Result<PairLink, AppErrorDto> {
    let invalid = |field: &str, message: &str| {
        let mut error = ProtoError::PairingInvalid(message.into()).to_app_error();
        error.path = Some(vec![unipi_proto::PathSegment::Key(field.into())]);
        error
    };
    let (ticket, code) = if input.trim().contains("://") {
        let url =
            url::Url::parse(input.trim()).map_err(|_| invalid("t", "Invalid pairing link"))?;
        if url.scheme() != "unipi" || url.host_str() != Some("pair") || !url.path().is_empty() {
            return Err(invalid("t", "Expected unipi://pair"));
        }
        let fields: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        (
            fields
                .get("t")
                .cloned()
                .ok_or_else(|| invalid("t", "Missing ticket"))?,
            fields
                .get("c")
                .cloned()
                .ok_or_else(|| invalid("c", "Missing code"))?,
        )
    } else {
        let fields: Vec<_> = input.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(invalid(
                if fields.len() == 1 { "c" } else { "t" },
                "Paste a ticket and code",
            ));
        }
        (fields[0].to_owned(), fields[1].to_owned())
    };
    parse_ticket(&ticket).map_err(|_| invalid("t", "Invalid endpoint ticket"))?;
    if code.len() != 10
        || !code
            .bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
    {
        return Err(invalid("c", "Code must be 10 Crockford base32 characters"));
    }
    Ok(PairLink { ticket, code })
}

/// Dials `addr`, timing out after 10 seconds (DESIGN.md's pairing
/// timeout budget applies equally to every iroh dial: `E_TIMEOUT`,
/// retryable).
pub async fn dial(endpoint: &Endpoint, addr: EndpointAddr) -> Result<Connection, AppErrorDto> {
    match tokio::time::timeout(Duration::from_secs(10), endpoint.connect(addr, ALPN)).await {
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(error)) => {
            Err(ProtoError::Transport(format!("failed to connect: {error}")).to_app_error())
        }
        Err(_elapsed) => {
            Err(ProtoError::Timeout("connect timed out after 10s".to_string()).to_app_error())
        }
    }
}

/// Opens one fresh bidirectional stream for a single logical
/// operation (DESIGN.md §5.4: "Every logical operation is one
/// bidirectional stream").
pub async fn open_op_stream(conn: &Connection) -> Result<(OpSend, OpRecv), AppErrorDto> {
    match tokio::time::timeout(Duration::from_secs(10), conn.open_bi()).await {
        Ok(Ok((send, recv))) => Ok((OpSend::Iroh(send), OpRecv::Iroh(recv))),
        Ok(Err(error)) => {
            Err(ProtoError::Transport(format!("failed to open stream: {error}")).to_app_error())
        }
        Err(_elapsed) => {
            Err(ProtoError::Timeout("open stream timed out after 10s".to_string()).to_app_error())
        }
    }
}

/// Classifies the currently selected path of `conn` for the UI's path
/// chip (DESIGN.md §3.3, §5.3), and returns the round-trip time of
/// that path when it is known.
pub fn selected_path(conn: &Connection) -> (PathKind, Option<Duration>) {
    let paths = conn.paths();
    let Some(selected) = paths.iter().find(|path| path.is_selected()) else {
        return (PathKind::Unknown, None);
    };

    let kind = match selected.remote_addr() {
        iroh::TransportAddr::Relay(_) => PathKind::Relay,
        iroh::TransportAddr::Ip(socket_addr) => {
            let local: Vec<std::net::IpAddr> = netdev::get_interfaces()
                .into_iter()
                .flat_map(|interface| {
                    interface
                        .ipv4
                        .into_iter()
                        .map(|net| std::net::IpAddr::V4(net.addr()))
                        .chain(
                            interface
                                .ipv6
                                .into_iter()
                                .map(|net| std::net::IpAddr::V6(net.addr())),
                        )
                })
                .collect();
            log::info!("UniPiPath selected={} local={:?}", socket_addr, local);
            classify_with_local(socket_addr.ip(), &local)
        }
        _ => PathKind::Unknown,
    };

    (kind, Some(selected.rtt()))
}

async fn bind_endpoint(
    app_data_dir: PathBuf,
    relay: Option<String>,
) -> Result<Endpoint, AppErrorDto> {
    let secret_key = load_or_create_secret_key(&app_data_dir)?;

    let mut builder = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()]);

    if let Some(url) = relay {
        let relay_url: RelayUrl = url.parse().map_err(|error| {
            ProtoError::Settings(format!("invalid relay url: {error}")).to_app_error()
        })?;
        let relay_map: RelayMap = std::iter::once(relay_url).collect();
        builder = builder.relay_mode(RelayMode::Custom(relay_map));
    }

    builder.bind().await.map_err(|error| {
        ProtoError::Transport(format!("failed to bind iroh endpoint: {error}")).to_app_error()
    })
}

/// Loads the persistent iroh secret key from the app data dir,
/// generating and saving a new one on first run. The endpoint id
/// (derived from this key) must be stable across restarts so a paired
/// host's allowlist entry keeps working.
fn load_or_create_secret_key(app_data_dir: &Path) -> Result<SecretKey, AppErrorDto> {
    let file = app_data_dir.join(SECRET_KEY_FILE);

    match std::fs::read(&file) {
        Ok(bytes) => {
            let array: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                ProtoError::Internal("iroh secret key file has the wrong length".to_string())
                    .to_app_error()
            })?;
            Ok(SecretKey::from_bytes(&array))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let secret_key = SecretKey::generate();
            save_secret_key(app_data_dir, &file, &secret_key)?;
            Ok(secret_key)
        }
        Err(error) => Err(
            ProtoError::Internal(format!("failed to read iroh secret key: {error}")).to_app_error(),
        ),
    }
}

fn save_secret_key(
    app_data_dir: &Path,
    file: &Path,
    secret_key: &SecretKey,
) -> Result<(), AppErrorDto> {
    std::fs::create_dir_all(app_data_dir).map_err(|error| {
        ProtoError::Internal(format!("failed to create app data dir: {error}")).to_app_error()
    })?;

    let tmp = app_data_dir.join(format!("{SECRET_KEY_FILE}.tmp"));
    std::fs::write(&tmp, secret_key.to_bytes()).map_err(|error| {
        ProtoError::Internal(format!("failed to write iroh secret key: {error}")).to_app_error()
    })?;
    std::fs::rename(&tmp, file).map_err(|error| {
        ProtoError::Internal(format!("failed to persist iroh secret key: {error}")).to_app_error()
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(file) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(file, perms);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_pair_link_fixtures() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../../protocol/fixtures/pairlink.json"))
                .unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let result = parse_pair_link(fixture["input"].as_str().unwrap());
            assert_eq!(
                result.is_ok(),
                fixture["ok"].as_bool().unwrap(),
                "{}",
                fixture["name"]
            );
            if let Err(error) = result {
                assert_eq!(
                    serde_json::to_value(error.path).unwrap(),
                    fixture["path"],
                    "{}",
                    fixture["name"]
                );
            }
        }
    }
    #[test]
    fn parse_ticket_rejects_garbage() {
        assert!(parse_ticket("not-a-ticket").is_err());
    }

    #[test]
    fn load_or_create_secret_key_is_stable_across_calls() {
        let dir = tempfile_dir();
        let first = load_or_create_secret_key(dir.path()).expect("create");
        let second = load_or_create_secret_key(dir.path()).expect("load");
        assert_eq!(first.to_bytes(), second.to_bytes());
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempfile_dir() -> TempDir {
        let mut dir = std::env::temp_dir();
        let unique = format!(
            "unipi-net-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        dir.push(unique);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}
