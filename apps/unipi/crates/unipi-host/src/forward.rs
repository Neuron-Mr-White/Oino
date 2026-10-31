//! Port forwarding (UNI-213, phase 1): `ports_list` and `tcp_forward`.
//!
//! `tcp_forward{port}` turns one iroh bistream into a raw TCP pipe to the
//! host's loopback (`127.0.0.1:port`, falling back to `[::1]:port`), like
//! `ssh -L`. The phone runs a loopback listener and opens one stream per
//! accepted connection, so HTTP, WebSockets and Vite HMR work unchanged.
//!
//! Security: OFF by default. Enabled only by the host owner in
//! `<host dir>/config.json` (read fresh on every request — no restart):
//!
//! ```json
//! { "forwarding": { "enabled": true, "ports": "any" } }
//! { "forwarding": { "enabled": true, "ports": [5173, 8080], "devices": ["Pixel"] } }
//! ```
//!
//! - `ports`: `"any"` (any loopback port) or an explicit list. Missing = none.
//! - `devices`: optional list of paired device ids (or id prefixes ≥ 8
//!   chars) or names; missing/empty = every paired device.
//! - The host never connects anywhere but loopback.
//!
//! `unipi-host forward enable|disable|status` edits this file.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use unipi_proto::{AppErrorCode, AppErrorDto, Control, ListeningPort, PortsListData};

use crate::protocol::write_control;

/// How the host may be asked to enable forwarding (shown in errors).
pub const ENABLE_HINT: &str =
    "On the PC run `unipi-host forward enable --any` (or `--port <n>`), no restart needed.";

/// Which ports may be forwarded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PortRule {
    /// Nothing (the default even when `enabled` is set without `ports`).
    #[default]
    None,
    /// Any loopback port.
    Any,
    /// Only these.
    List(Vec<u16>),
}

impl Serialize for PortRule {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PortRule::Any => serializer.serialize_str("any"),
            PortRule::None => Vec::<u16>::new().serialize(serializer),
            PortRule::List(ports) => ports.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PortRule {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Word(String),
            List(Vec<u16>),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Word(word) if word == "any" => Ok(PortRule::Any),
            Raw::Word(other) => Err(serde::de::Error::custom(format!(
                "ports must be \"any\" or a list of port numbers, got {other:?}"
            ))),
            Raw::List(list) if list.is_empty() => Ok(PortRule::None),
            Raw::List(list) => Ok(PortRule::List(list)),
        }
    }
}

/// The `forwarding` block of `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForwardingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ports: PortRule,
    /// Paired device ids / id prefixes / names allowed to forward;
    /// empty = all paired devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
}

/// `<host dir>/config.json`. Unknown keys are preserved on rewrite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default)]
    pub forwarding: ForwardingConfig,
    /// UNI-214: `unipi-host push` (off by default).
    #[serde(default)]
    pub push: crate::push::PushConfig,
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl HostConfig {
    /// Reads the config; a missing file is the default (forwarding off).
    pub fn load(path: &Path) -> anyhow::Result<HostConfig> {
        Ok(crate::data_dir::read_json(path)?.unwrap_or_default())
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        crate::data_dir::write_json(path, self)
    }
}

fn disabled(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_FORWARD_DISABLED, message, false).with_hint(ENABLE_HINT)
}

impl ForwardingConfig {
    /// Is this paired device allowed to forward at all?
    pub fn device_allowed(&self, device_id: &str, device_name: Option<&str>) -> bool {
        self.devices.is_empty()
            || device_id == crate::protocol::LOCAL_DEVICE_ID
            || self.devices.iter().any(|entry| {
                entry == device_id
                    || (entry.len() >= 8 && device_id.starts_with(entry.as_str()))
                    || Some(entry.as_str()) == device_name
            })
    }

    pub fn port_allowed(&self, port: u16) -> bool {
        match &self.ports {
            PortRule::None => false,
            PortRule::Any => true,
            PortRule::List(ports) => ports.contains(&port),
        }
    }

    /// The gate every forwarding request goes through.
    pub fn check_device(
        &self,
        device_id: &str,
        device_name: Option<&str>,
    ) -> Result<(), AppErrorDto> {
        if !self.enabled {
            return Err(disabled("Port forwarding is off on this PC"));
        }
        if matches!(self.ports, PortRule::None) {
            return Err(disabled(
                "Port forwarding is on but no ports are allowed on this PC",
            ));
        }
        if !self.device_allowed(device_id, device_name) {
            return Err(disabled(
                "This phone is not allowed to forward ports on this PC",
            ));
        }
        Ok(())
    }

    pub fn check_port(&self, port: u16) -> Result<(), AppErrorDto> {
        if self.port_allowed(port) {
            Ok(())
        } else {
            Err(AppErrorDto::new(
                AppErrorCode::E_UNAUTHORIZED,
                format!("Port {port} is not in this PC's forwarding allow-list"),
                false,
            )
            .with_hint(format!(
                "On the PC run `unipi-host forward enable --port {port}`."
            )))
        }
    }
}

// ---------------------------------------------------------------- /proc

/// One LISTEN row from `/proc/net/tcp{,6}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcListen {
    pub addr: IpAddr,
    pub port: u16,
    pub inode: u64,
}

fn parse_v4(hex: &str) -> Option<Ipv4Addr> {
    let raw = u32::from_str_radix(hex, 16).ok()?;
    // The kernel prints the in-memory (network order) u32 as host-endian hex.
    Some(Ipv4Addr::from(raw.to_ne_bytes()))
}

fn parse_v6(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for word in 0..4 {
        let raw = u32::from_str_radix(&hex[word * 8..word * 8 + 8], 16).ok()?;
        bytes[word * 4..word * 4 + 4].copy_from_slice(&raw.to_ne_bytes());
    }
    Some(Ipv6Addr::from(bytes))
}

/// Parses the text of `/proc/net/tcp` or `/proc/net/tcp6`, keeping only
/// sockets in state LISTEN (`0A`). Malformed rows are skipped.
pub fn parse_proc_net_tcp(text: &str) -> Vec<ProcListen> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != "0A" {
                return None;
            }
            let (ip_hex, port_hex) = fields[1].split_once(':')?;
            let addr = match ip_hex.len() {
                8 => IpAddr::V4(parse_v4(ip_hex)?),
                32 => IpAddr::V6(parse_v6(ip_hex)?),
                _ => return None,
            };
            let port = u16::from_str_radix(port_hex, 16).ok()?;
            let inode = fields[9].parse().ok()?;
            Some(ProcListen { addr, port, inode })
        })
        .collect()
}

/// Can a socket bound to `addr` be reached via 127.0.0.1 / ::1?
pub fn loopback_reachable(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6
                    .to_ipv4_mapped()
                    .map(|v4| v4.is_loopback() || v4.is_unspecified())
                    .unwrap_or(false)
        }
    }
}

/// Web-server-looking heuristic from the process name/cmd and port (no
/// probing: poking arbitrary services with HTTP can upset them).
pub fn looks_http(port: u16, process: Option<&str>, cmd: Option<&str>) -> bool {
    const PROCS: &[&str] = &[
        "node",
        "deno",
        "bun",
        "vite",
        "next",
        "nuxt",
        "python",
        "python3",
        "uvicorn",
        "gunicorn",
        "flask",
        "django",
        "ruby",
        "rails",
        "puma",
        "php",
        "php-fpm",
        "nginx",
        "caddy",
        "httpd",
        "apache2",
        "lighttpd",
        "hugo",
        "jekyll",
        "java",
        "dotnet",
        "esbuild",
        "webpack",
        "astro",
        "storybook",
        "jupyter",
        "code-server",
        "grafana",
        "miniserve",
        "http-server",
        "serve",
    ];
    const CMD_HINTS: &[&str] = &[
        "vite",
        "next dev",
        "next start",
        "http.server",
        "webpack",
        "runserver",
        "uvicorn",
        "astro",
        "storybook",
        "jupyter",
        "hugo server",
        "http-server",
        "rails s",
        "flask run",
        "nuxt",
        "npm run dev",
        "pnpm dev",
        "yarn dev",
        "bun dev",
        "mkdocs serve",
        "docusaurus",
    ];
    let proc_hit = process
        .map(|p| PROCS.contains(&p.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    let cmd_hit = cmd
        .map(|c| CMD_HINTS.iter().any(|hint| c.contains(hint)))
        .unwrap_or(false);
    let port_hit = matches!(port, 80 | 443 | 1420 | 3000..=3010 | 4000 | 4200 | 4321 | 5173..=5180 | 6006 | 8000 | 8080..=8089 | 8443 | 8787..=8799 | 8888);
    proc_hit || cmd_hit || port_hit
}

/// Maps socket inode → pid by scanning `/proc/<pid>/fd` (only our own
/// processes are readable; others just stay unnamed).
fn socket_owners(proc_root: &Path, wanted: &std::collections::HashSet<u64>) -> HashMap<u64, u32> {
    let mut owners = HashMap::new();
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return owners;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let target = target.to_string_lossy();
            if let Some(inode) = target
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
            {
                if let Ok(inode) = inode.parse::<u64>() {
                    if wanted.contains(&inode) {
                        owners.insert(inode, pid);
                    }
                }
            }
        }
        if owners.len() == wanted.len() {
            break;
        }
    }
    owners
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Builds the `ports_list` answer from a `/proc`-shaped root (tests pass a
/// fake tree). One entry per port (v4/v6 duplicates merged), sorted by port.
pub fn list_ports(proc_root: &Path, config: &ForwardingConfig) -> PortsListData {
    let mut rows = Vec::new();
    for name in ["tcp", "tcp6"] {
        if let Ok(text) = std::fs::read_to_string(proc_root.join("net").join(name)) {
            rows.extend(parse_proc_net_tcp(&text));
        }
    }
    rows.retain(|row| loopback_reachable(&row.addr));
    let wanted = rows
        .iter()
        .map(|row| row.inode)
        .filter(|inode| *inode != 0)
        .collect();
    let owners = socket_owners(proc_root, &wanted);
    let mut by_port: BTreeMap<u16, ListeningPort> = BTreeMap::new();
    for row in rows {
        let pid = owners.get(&row.inode).copied();
        let argv: Vec<String> = pid
            .and_then(|pid| read_trimmed(&proc_root.join(pid.to_string()).join("cmdline")))
            .map(|raw| {
                raw.split('\0')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let cmd = (!argv.is_empty()).then(|| argv.join(" ").chars().take(200).collect::<String>());
        // argv[0]'s basename beats `comm`: node renames its main thread
        // "MainThread", and comm is cut at 15 bytes.
        let process = argv
            .first()
            .and_then(|arg0| arg0.rsplit('/').next())
            .map(str::to_string)
            .filter(|name| !name.is_empty())
            .or_else(|| {
                pid.and_then(|pid| read_trimmed(&proc_root.join(pid.to_string()).join("comm")))
            });
        let entry = by_port.entry(row.port).or_insert_with(|| ListeningPort {
            port: row.port,
            bind: row.addr.to_string(),
            pid: None,
            process: None,
            cmd: None,
            http: false,
            allowed: config.port_allowed(row.port),
        });
        // Prefer the IPv4 bind for display; fill owner info from any row.
        if row.addr.is_ipv4() && !entry.bind.contains('.') {
            entry.bind = row.addr.to_string();
        }
        if entry.pid.is_none() && pid.is_some() {
            entry.pid = pid;
            entry.process = process;
            entry.cmd = cmd;
        }
    }
    let ports = by_port
        .into_values()
        .map(|mut port| {
            port.http = looks_http(port.port, port.process.as_deref(), port.cmd.as_deref());
            port
        })
        .collect();
    PortsListData {
        ports,
        any: matches!(config.ports, PortRule::Any),
        allowed: match &config.ports {
            PortRule::List(list) => list.clone(),
            _ => Vec::new(),
        },
    }
}

// ---------------------------------------------------------------- streams

/// Connects to the loopback port, IPv4 first then IPv6.
pub async fn connect_loopback(port: u16) -> Result<tokio::net::TcpStream, AppErrorDto> {
    let mut last = None;
    for ip in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ] {
        match tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(SocketAddr::new(ip, port)),
        )
        .await
        {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(error)) => last = Some(error.to_string()),
            Err(_) => last = Some("timed out".into()),
        }
    }
    Err(AppErrorDto::new(
        AppErrorCode::E_NOT_FOUND,
        format!("Nothing is listening on port {port} on this PC"),
        false,
    )
    .with_detail(serde_json::json!({ "error": last })))
}

/// Pipes bytes both ways until both directions finish. Each direction
/// half-closes its destination when its source ends (so a server closing
/// its socket reaches the phone, and vice versa).
pub async fn pipe<R, W, S>(read: &mut R, write: &mut W, socket: S) -> (u64, u64)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut socket_read, mut socket_write) = tokio::io::split(socket);
    let up = async {
        let n = tokio::io::copy(read, &mut socket_write).await.unwrap_or(0);
        let _ = socket_write.shutdown().await;
        n
    };
    let down = async {
        let n = tokio::io::copy(&mut socket_read, write).await.unwrap_or(0);
        let _ = write.shutdown().await;
        n
    };
    tokio::join!(up, down)
}

/// Handles one `tcp_forward` stream after the hello: gate, connect, `ok{}`,
/// then raw bytes.
pub async fn serve_forward<R, W>(
    config: &ForwardingConfig,
    device_id: &str,
    device_name: Option<&str>,
    read: &mut R,
    write: &mut W,
    port: u16,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    config.check_device(device_id, device_name)?;
    config.check_port(port)?;
    let socket = connect_loopback(port).await?;
    write_control(write, &Control::Ok { data: None })
        .await
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true))?;
    let (up, down) = pipe(read, write, socket).await;
    tracing::info!(op = "tcp_forward", port, up, down, "closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F6F 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0000000000000000 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 222 1 0000000000000000 100 0 0 10 0
   2: 0101A8C0:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 333 1 0000000000000000 100 0 0 10 0
   3: 0100007F:1F6F 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 444 1 0000000000000000 100 0 0 10 0
   garbage line
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1435 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 555 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 666 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:22B3 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 777 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn parses_listen_rows_v4_and_v6() {
        let v4 = parse_proc_net_tcp(TCP);
        assert_eq!(v4.len(), 3, "{v4:?}");
        assert_eq!(
            v4[0],
            ProcListen {
                addr: "127.0.0.1".parse().unwrap(),
                port: 8047,
                inode: 111
            }
        );
        assert_eq!(v4[1].addr, "0.0.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(v4[1].port, 22);
        assert_eq!(v4[2].addr, "192.168.1.1".parse::<IpAddr>().unwrap());
        let v6 = parse_proc_net_tcp(TCP6);
        assert_eq!(v6.len(), 3);
        assert_eq!(v6[0].addr, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(v6[0].port, 5173);
        assert_eq!(v6[1].addr, "::".parse::<IpAddr>().unwrap());
        assert_eq!(v6[2].addr, "::ffff:127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(v6[2].port, 8883);
    }

    #[test]
    fn loopback_reachability() {
        for ok in [
            "127.0.0.1",
            "127.0.0.53",
            "0.0.0.0",
            "::1",
            "::",
            "::ffff:127.0.0.1",
        ] {
            assert!(loopback_reachable(&ok.parse().unwrap()), "{ok}");
        }
        for no in ["192.168.1.1", "10.0.0.2", "fe80::1"] {
            assert!(!loopback_reachable(&no.parse().unwrap()), "{no}");
        }
    }

    #[test]
    fn real_proc_parses_without_panicking() {
        if let Ok(text) = std::fs::read_to_string("/proc/net/tcp") {
            let _ = parse_proc_net_tcp(&text);
        }
    }

    #[test]
    fn list_ports_from_fake_proc_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("net")).unwrap();
        std::fs::write(root.join("net/tcp"), TCP).unwrap();
        std::fs::write(root.join("net/tcp6"), TCP6).unwrap();
        // pid 4242 owns inode 555 (port 5173): node vite
        let fd = root.join("4242/fd");
        std::fs::create_dir_all(&fd).unwrap();
        std::os::unix::fs::symlink("socket:[555]", fd.join("7")).unwrap();
        std::os::unix::fs::symlink("/dev/null", fd.join("0")).unwrap();
        std::fs::write(root.join("4242/comm"), "MainThread\n").unwrap();
        std::fs::write(
            root.join("4242/cmdline"),
            "node\x00/x/node_modules/.bin/vite\x00--port\x005173\x00",
        )
        .unwrap();
        let config = ForwardingConfig {
            enabled: true,
            ports: PortRule::List(vec![5173]),
            devices: vec![],
        };
        let data = list_ports(root, &config);
        let ports: Vec<u16> = data.ports.iter().map(|p| p.port).collect();
        // 8000 bound to 192.168.1.1 is not reachable over loopback.
        assert_eq!(ports, vec![22, 5173, 8047, 8883]);
        let vite = data.ports.iter().find(|p| p.port == 5173).unwrap();
        assert_eq!(vite.pid, Some(4242));
        assert_eq!(vite.process.as_deref(), Some("node"));
        assert_eq!(
            vite.cmd.as_deref(),
            Some("node /x/node_modules/.bin/vite --port 5173")
        );
        assert!(vite.http);
        assert!(vite.allowed);
        let ssh = data.ports.iter().find(|p| p.port == 22).unwrap();
        assert_eq!(ssh.bind, "0.0.0.0", "v4 bind preferred over ::");
        assert!(!ssh.http);
        assert!(!ssh.allowed);
        assert!(!data.any);
        assert_eq!(data.allowed, vec![5173]);
    }

    #[test]
    fn config_parsing_and_defaults() {
        let off = HostConfig::default();
        assert!(!off.forwarding.enabled);
        assert!(off.forwarding.check_device("abc", None).is_err());
        let any: HostConfig =
            serde_json::from_str(r#"{"forwarding":{"enabled":true,"ports":"any"},"other":1}"#)
                .unwrap();
        assert_eq!(any.forwarding.ports, PortRule::Any);
        assert!(any.forwarding.check_device("abc", None).is_ok());
        assert!(any.forwarding.check_port(1).is_ok());
        // Unknown keys survive a rewrite.
        let text = serde_json::to_string(&any).unwrap();
        assert!(
            text.contains("\"other\":1") && text.contains("\"any\""),
            "{text}"
        );
        let list: HostConfig =
            serde_json::from_str(r#"{"forwarding":{"enabled":true,"ports":[5173]}}"#).unwrap();
        assert!(list.forwarding.check_port(5173).is_ok());
        let denied = list.forwarding.check_port(22).unwrap_err();
        assert_eq!(denied.code, AppErrorCode::E_UNAUTHORIZED);
        assert!(serde_json::from_str::<HostConfig>(r#"{"forwarding":{"ports":"all"}}"#).is_err());
        // enabled without ports = nothing allowed, with a clear reason.
        let empty: HostConfig = serde_json::from_str(r#"{"forwarding":{"enabled":true}}"#).unwrap();
        let error = empty.forwarding.check_device("abc", None).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_FORWARD_DISABLED);
        assert!(error.message.as_str().contains("no ports"));
    }

    #[test]
    fn disabled_error_explains_how_to_enable() {
        let error = ForwardingConfig::default()
            .check_device("abc", None)
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_FORWARD_DISABLED);
        assert!(error.hint.unwrap().contains("unipi-host forward enable"));
    }

    #[test]
    fn per_device_gate() {
        let config = ForwardingConfig {
            enabled: true,
            ports: PortRule::Any,
            devices: vec!["Pixel".into(), "abcdef12".into()],
        };
        assert!(
            config.check_device("abcdef1234567890", None).is_ok(),
            "id prefix"
        );
        assert!(config.check_device("ffff", Some("Pixel")).is_ok(), "name");
        let error = config.check_device("ffff", Some("Redmi")).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_FORWARD_DISABLED);
        // Short prefixes don't match (avoid "a" granting everyone).
        let short = ForwardingConfig {
            devices: vec!["ab".into()],
            ..config
        };
        assert!(short.check_device("abcdef", None).is_err());
    }

    #[test]
    fn http_heuristic() {
        assert!(looks_http(5173, Some("node"), None));
        assert!(looks_http(
            12345,
            Some("python3"),
            Some("python3 -m http.server 12345")
        ));
        assert!(looks_http(8080, None, None));
        assert!(!looks_http(22, Some("sshd"), Some("sshd: /usr/sbin/sshd")));
        assert!(!looks_http(5432, Some("postgres"), None));
        // Real false positives seen on a desktop: these aren't web apps.
        assert!(!looks_http(
            21118,
            Some("rustdesk"),
            Some("/usr/share/rustdesk/rustdesk --server")
        ));
        assert!(!looks_http(
            5037,
            Some("adb"),
            Some("adb -L tcp:5037 fork-server server")
        ));
        assert!(!looks_http(
            22000,
            Some("syncthing"),
            Some("/usr/bin/syncthing serve --no-browser")
        ));
    }

    #[tokio::test]
    async fn pipe_copies_both_ways_and_half_closes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            socket.read_to_end(&mut got).await.unwrap();
            socket.write_all(b"echo:").await.unwrap();
            socket.write_all(&got).await.unwrap();
            got
        });
        let socket = connect_loopback(port).await.unwrap();
        // "phone" side: an in-memory duplex standing in for the iroh stream.
        let (phone, host_side) = tokio::io::duplex(64);
        let (mut host_read, mut host_write) = tokio::io::split(host_side);
        let piping =
            tokio::spawn(async move { pipe(&mut host_read, &mut host_write, socket).await });
        let (mut phone_read, mut phone_write) = tokio::io::split(phone);
        let payload = vec![7u8; 100_000];
        let writer = {
            let payload = payload.clone();
            tokio::spawn(async move {
                phone_write.write_all(&payload).await.unwrap();
                phone_write.shutdown().await.unwrap();
            })
        };
        let mut back = Vec::new();
        phone_read.read_to_end(&mut back).await.unwrap();
        writer.await.unwrap();
        assert_eq!(&back[..5], b"echo:");
        assert_eq!(&back[5..], &payload[..]);
        assert_eq!(server.await.unwrap(), payload);
        let (up, down) = piping.await.unwrap();
        assert_eq!(up, 100_000);
        assert_eq!(down, 100_005);
    }

    #[tokio::test]
    async fn connect_loopback_reports_closed_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = connect_loopback(port).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_forward_refuses_when_disabled_or_not_allowed() {
        let (mut read, _) = tokio::io::duplex(8);
        let mut sink = Vec::new();
        let off = ForwardingConfig::default();
        let error = serve_forward(&off, "abc", None, &mut read, &mut sink, 80)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_FORWARD_DISABLED);
        let list = ForwardingConfig {
            enabled: true,
            ports: PortRule::List(vec![5173]),
            devices: vec![],
        };
        let error = serve_forward(&list, "abc", None, &mut read, &mut sink, 80)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_UNAUTHORIZED);
        assert!(sink.is_empty(), "nothing written before the gate passes");
    }
}
