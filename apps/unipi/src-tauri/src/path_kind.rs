//! Classification of the network path an iroh connection is actually
//! using, for the UI's path chip (DESIGN.md §3.3, §5.3): LAN
//! (RFC1918/link-local/loopback/ULA), Tailnet (100.64.0.0/10), Direct
//! (any other IP) or Relay.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

/// The kind of network path a connection is currently using.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// The user's own tailnet (CGNAT range `100.64.0.0/10`), reached
    /// as a plain direct IP address.
    Tailnet,
    /// A private LAN address (RFC1918, link-local, loopback or IPv6
    /// ULA), reached as a plain direct IP address.
    Lan,
    /// Any other direct IP address (a hole-punched or otherwise
    /// routable internet path).
    Direct,
    /// The iroh relay, used as a last resort.
    Relay,
    /// UNI-231: this PC's own host over its local socket (no network).
    Local,
    /// The path couldn't be classified (for example, a `Custom`
    /// transport address, which iroh M1 doesn't use).
    Unknown,
}

/// Classifies a selected path's remote IP address. Pass `None` for a
/// relay path.
pub fn classify_ip(addr: Option<IpAddr>) -> PathKind {
    let Some(addr) = addr else {
        return PathKind::Relay;
    };

    match addr {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(addr: Ipv4Addr) -> PathKind {
    if is_tailscale_cgnat(addr) {
        return PathKind::Tailnet;
    }
    if addr.is_private() || addr.is_link_local() || addr.is_loopback() {
        return PathKind::Lan;
    }
    PathKind::Direct
}

fn classify_v6(addr: Ipv6Addr) -> PathKind {
    if addr.is_loopback() {
        return PathKind::Lan;
    }
    // IPv6 Unique Local Address: fc00::/7.
    if (addr.segments()[0] & 0xfe00) == 0xfc00 {
        return PathKind::Lan;
    }
    // IPv6 link-local: fe80::/10.
    if (addr.segments()[0] & 0xffc0) == 0xfe80 {
        return PathKind::Lan;
    }
    // An IPv4-mapped address embeds a v4 address; classify that instead.
    if let Some(v4) = addr.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    PathKind::Direct
}

/// Tailscale's shared CGNAT range: `100.64.0.0/10` (DESIGN.md §3.3, §5.3).
fn is_tailscale_cgnat(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000
}

pub fn classify_with_local(remote: IpAddr, local: &[IpAddr]) -> PathKind {
    let base = classify_ip(Some(remote));
    if base != PathKind::Direct {
        return base;
    }
    if local.iter().any(|address| match (remote, *address) {
        (IpAddr::V4(a), IpAddr::V4(b)) => a.octets()[..3] == b.octets()[..3],
        (IpAddr::V6(a), IpAddr::V6(b)) => a.octets()[..8] == b.octets()[..8],
        _ => false,
    }) {
        PathKind::Lan
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::net::Ipv6Addr;

    #[test]
    fn shared_subnets_are_lan() {
        assert_eq!(
            classify_with_local(
                "2001:db8:1:2::8".parse().unwrap(),
                &["2001:db8:1:2::9".parse().unwrap()]
            ),
            PathKind::Lan
        );
        assert_eq!(
            classify_with_local("8.8.8.1".parse().unwrap(), &["8.8.8.2".parse().unwrap()]),
            PathKind::Lan
        );
        assert_eq!(
            classify_with_local(
                "2001:db8:1:3::8".parse().unwrap(),
                &["2001:db8:1:2::9".parse().unwrap()]
            ),
            PathKind::Direct
        );
    }
    #[test]
    fn relay_path_has_no_ip() {
        assert_eq!(classify_ip(None), PathKind::Relay);
    }

    #[test]
    fn classifies_tailnet_range() {
        assert_eq!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)))),
            PathKind::Tailnet
        );
        assert_eq!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255)))),
            PathKind::Tailnet
        );
    }

    #[test]
    fn rejects_cgnat_lookalikes_outside_tailscale_range() {
        // 100.63.x.x and 100.128.x.x look similar but are outside 100.64.0.0/10.
        assert_ne!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1)))),
            PathKind::Tailnet
        );
        assert_ne!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1)))),
            PathKind::Tailnet
        );
    }

    #[test]
    fn classifies_rfc1918_as_lan() {
        for ip in [
            Ipv4Addr::new(10, 0, 0, 5),
            Ipv4Addr::new(172, 16, 0, 5),
            Ipv4Addr::new(192, 168, 1, 5),
        ] {
            assert_eq!(classify_ip(Some(IpAddr::V4(ip))), PathKind::Lan);
        }
    }

    #[test]
    fn classifies_link_local_and_loopback_as_lan() {
        assert_eq!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)))),
            PathKind::Lan
        );
        assert_eq!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            PathKind::Lan
        );
        assert_eq!(
            classify_ip(Some(IpAddr::V6(Ipv6Addr::LOCALHOST))),
            PathKind::Lan
        );
    }

    #[test]
    fn classifies_ipv6_ula_and_link_local_as_lan() {
        let ula: Ipv6Addr = "fd00::1".parse().unwrap();
        assert_eq!(classify_ip(Some(IpAddr::V6(ula))), PathKind::Lan);
        let link_local: Ipv6Addr = "fe80::1".parse().unwrap();
        assert_eq!(classify_ip(Some(IpAddr::V6(link_local))), PathKind::Lan);
    }

    #[test]
    fn classifies_other_addresses_as_direct() {
        assert_eq!(
            classify_ip(Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)))),
            PathKind::Direct
        );
        let global: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert_eq!(classify_ip(Some(IpAddr::V6(global))), PathKind::Direct);
    }

    #[test]
    fn ipv4_mapped_ipv6_classifies_as_its_v4_address() {
        let mapped: Ipv6Addr = "::ffff:192.168.1.1".parse().unwrap();
        assert_eq!(classify_ip(Some(IpAddr::V6(mapped))), PathKind::Lan);
    }
}

#[cfg(test)]
mod wire_tests {
    use super::PathKind;
    /// UNI-231: the TS twin (`apps/mobile/src/lib/schemas.ts` PathKindSchema)
    /// lists exactly these strings — keep both in step.
    #[test]
    fn path_kinds_serialize_as_the_ui_expects() {
        let all = [
            PathKind::Lan,
            PathKind::Tailnet,
            PathKind::Direct,
            PathKind::Relay,
            PathKind::Local,
            PathKind::Unknown,
        ];
        let names: Vec<String> = all
            .iter()
            .map(|k| {
                serde_json::to_value(k)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            ["lan", "tailnet", "direct", "relay", "local", "unknown"]
        );
        let ts = include_str!("../../src/lib/schemas.ts");
        assert!(
            ts.contains("z.enum(['lan', 'tailnet', 'direct', 'relay', 'local', 'unknown'])"),
            "schemas.ts PathKindSchema drifted"
        );
    }
}
