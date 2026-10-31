//! iroh endpoint bootstrap: relay-only transport (no local IP
//! transports — see `docs/m1/host-notes.md` for the evidence this
//! actually routes traffic end to end over an n0 relay), ALPN
//! `unipi/oino/1` (v2 — distinct from the old unipi's `unipi/app/1`, so
//! the two hosts and their phones never cross), and `endpoint.json`
//! caching with a freshness watcher so `unipi-host pair` always
//! displays an up to date ticket.

use std::path::{Path, PathBuf};

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey, Watcher};
use iroh_tickets::endpoint::EndpointTicket;
use serde::{Deserialize, Serialize};

/// ALPN for `unipi/oino/1` (v2; the old unipi app-host used `unipi/app/1` —
/// spec §5.1 coexistence table).
pub const ALPN: &[u8] = b"unipi/oino/1";

/// Builds a relay-only endpoint: the n0 default relays provide
/// rendezvous and fallback relaying, but no local IP transport is
/// bound, so the endpoint never advertises (or dials) a LAN/direct
/// address. See `docs/m1/host-notes.md` §"Relay-only evidence".
pub async fn bind_relay_only(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
) -> anyhow::Result<Endpoint> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .clear_ip_transports()
        .alpns(alpns)
        .bind()
        .await?;
    Ok(endpoint)
}

/// The cached `endpoint.json`: the host's current address and
/// serialized ticket, refreshed whenever the endpoint's observed
/// address changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointCache {
    pub addr: EndpointAddr,
    pub ticket: String,
}

impl EndpointCache {
    pub fn from_addr(addr: EndpointAddr) -> Self {
        let ticket = EndpointTicket::new(addr.clone()).to_string();
        EndpointCache { addr, ticket }
    }
}

/// Resolves the endpoint.json path for a data directory.
pub fn endpoint_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("endpoint.json")
}

/// Writes the endpoint's current address to `path`, then spawns a
/// background task that rewrites it on every subsequent address
/// change for as long as `endpoint` (or any clone) is alive. This
/// keeps the ticket shown by `unipi-host pair` fresh across relay
/// changes without the caller needing to poll.
pub async fn persist_and_watch(
    endpoint: &Endpoint,
    path: PathBuf,
) -> anyhow::Result<EndpointCache> {
    let addr = endpoint.watch_addr().get();
    let cache = EndpointCache::from_addr(addr);
    crate::data_dir::write_json(&path, &cache)?;

    let endpoint = endpoint.clone();
    tokio::spawn(async move {
        let mut watcher = endpoint.watch_addr();
        while let Ok(addr) = Watcher::updated(&mut watcher).await {
            let cache = EndpointCache::from_addr(addr);
            if let Err(error) = crate::data_dir::write_json(&path, &cache) {
                tracing::warn!(?error, "failed to persist refreshed endpoint.json");
            }
        }
    });

    Ok(cache)
}

/// Reads a previously cached `endpoint.json`, if present.
pub fn read_cached(path: &Path) -> anyhow::Result<Option<EndpointCache>> {
    crate::data_dir::read_json(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binds_relay_only_and_persists_a_fresh_ticket() {
        let secret = SecretKey::generate();
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("bind");

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = endpoint_cache_path(tmp.path());
        let cache = persist_and_watch(&endpoint, path.clone())
            .await
            .expect("persist_and_watch");

        assert!(!cache.ticket.is_empty());
        assert!(cache.ticket.starts_with("endpoint"));

        let from_disk = read_cached(&path).expect("read_cached").expect("some");
        assert_eq!(from_disk, cache);

        // No local IP transport should be advertised.
        assert!(cache.addr.addrs.iter().any(|a| a.is_ip()));

        endpoint.close().await;
    }
}
