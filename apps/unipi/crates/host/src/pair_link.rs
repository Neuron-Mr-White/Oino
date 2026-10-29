//! `unipi-host pair` output: a compact pairing link and a terminal QR
//! code that phone scanners (ML Kit) can actually read.
//!
//! Two things made the original QR unscannable:
//! 1. The link carried the full cached ticket — every local IP
//!    (docker bridges, tailnet, public) plus the relay, ~580 chars —
//!    so the QR was a very high version that wrapped in most terminals.
//!    Host and app both use the n0 preset (relays + DNS/pkarr address
//!    lookup), so the endpoint id plus the home relay URL is enough to
//!    dial; direct/LAN paths are discovered by hole punching after the
//!    relay connection is up.
//! 2. It was drawn with default colours, so on a dark terminal the
//!    modules came out *inverted* (light on dark), which many
//!    scanners refuse. We draw light modules (and the quiet zone) as
//!    blocks and dark modules as blank cells, and — on a TTY — pin
//!    white-on-black colours so it looks the same on light themes.

use iroh::{EndpointAddr, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use qrcode::render::unicode::Dense1x2;
use qrcode::{EcLevel, QrCode};

/// A minimal ticket for the pairing link: the endpoint id plus its
/// relay URL(s), no IP addresses. Falls back to the full address when
/// the endpoint has no relay (relays disabled), so the link still
/// works on a LAN-only setup.
pub fn pairing_ticket(addr: &EndpointAddr) -> String {
    let relays: Vec<TransportAddr> = addr
        .addrs
        .iter()
        .filter(|a| a.is_relay())
        .cloned()
        .collect();
    let minimal = if relays.is_empty() {
        addr.clone()
    } else {
        EndpointAddr::from_parts(addr.id, relays)
    };
    EndpointTicket::new(minimal).to_string()
}

/// Builds the `unipi://pair` deep link.
pub fn pairing_link(ticket: &str, code: &str, host_name: &str) -> anyhow::Result<String> {
    let mut link = url::Url::parse("unipi://pair")?;
    link.query_pairs_mut()
        .append_pair("t", ticket)
        .append_pair("c", code)
        .append_pair("n", host_name);
    Ok(link.to_string())
}

/// A rendered terminal QR code.
pub struct TerminalQr {
    /// Lines of text (without colour escapes).
    pub lines: Vec<String>,
    /// QR version (1–40).
    pub version: i16,
    /// Width in modules, without the quiet zone.
    pub modules: usize,
}

impl TerminalQr {
    /// Width in terminal columns (including the quiet zone).
    pub fn columns(&self) -> usize {
        self.lines.first().map_or(0, |l| l.chars().count())
    }

    /// The printable form. With `color`, each line is pinned to bright
    /// white on black so the code renders correctly on any theme.
    pub fn to_terminal_string(&self, color: bool) -> String {
        if !color {
            return self.lines.join("\n");
        }
        self.lines
            .iter()
            .map(|line| format!("\x1b[97;40m{line}\x1b[0m"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Renders `data` as a low-EC QR code for a terminal: two modules per
/// character cell (Unicode half blocks, so modules stay square), a
/// 4-module quiet zone, light modules drawn as blocks.
pub fn render_terminal_qr(data: &str) -> anyhow::Result<TerminalQr> {
    let qr = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::L)?;
    let version = match qr.version() {
        qrcode::Version::Normal(v) => v,
        qrcode::Version::Micro(v) => v,
    };
    let modules = qr.width();
    let text = qr
        .render::<Dense1x2>()
        // Swap: "dark" glyphs are the filled blocks, which a dark
        // terminal shows *light*. Light modules → blocks.
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    Ok(TerminalQr {
        lines: text.lines().map(str::to_owned).collect(),
        version,
        modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn full_addr() -> EndpointAddr {
        let id = SecretKey::generate().public();
        let mut addr = EndpointAddr::new(id)
            .with_relay_url("https://aps1-1.relay.n0.iroh.link./".parse().unwrap());
        for i in 0..20u8 {
            addr = addr.with_ip_addr(format!("172.{i}.0.1:49513").parse().unwrap());
        }
        addr
    }

    #[test]
    fn pairing_ticket_keeps_only_id_and_relay() {
        let addr = full_addr();
        let full = EndpointTicket::new(addr.clone()).to_string();
        let minimal = pairing_ticket(&addr);
        assert!(minimal.len() < 140, "{} chars", minimal.len());
        assert!(minimal.len() < full.len() / 2);
        let parsed: EndpointTicket = minimal.parse().unwrap();
        let back = parsed.endpoint_addr();
        assert_eq!(back.id, addr.id);
        assert_eq!(back.ip_addrs().count(), 0);
        assert_eq!(back.relay_urls().count(), 1);
    }

    #[test]
    fn pairing_ticket_without_relay_keeps_ip_addrs() {
        let id = SecretKey::generate().public();
        let addr = EndpointAddr::new(id).with_ip_addr("192.168.1.2:4000".parse().unwrap());
        let parsed: EndpointTicket = pairing_ticket(&addr).parse().unwrap();
        assert_eq!(parsed.endpoint_addr().ip_addrs().count(), 1);
    }

    #[test]
    fn link_qr_is_small_and_light_modules_are_blocks() {
        let ticket = pairing_ticket(&full_addr());
        let link = pairing_link(&ticket, "ABCDEFGH12", "workstation").unwrap();
        let qr = render_terminal_qr(&link).unwrap();
        assert!(qr.version <= 9, "version {}", qr.version);
        // Quiet zone (light) → the first row is solid full blocks.
        assert!(qr.lines[0].chars().all(|c| c == '█'), "{:?}", qr.lines[0]);
        assert_eq!(qr.columns(), qr.modules + 8);
        assert!(qr.columns() <= 70);
        let colored = qr.to_terminal_string(true);
        assert!(colored.starts_with("\x1b[97;40m"));
    }
}
