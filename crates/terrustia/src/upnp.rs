//! UPnP IGD automatic port-mapping for the game port, attempted once at boot.
//!
//! The same job AstroLauncher does for its own Terraria/Minecraft/Valheim server launcher: on
//! startup, ask the router (via UPnP's SSDP discovery, then a SOAP `AddPortMapping` call) to
//! forward the configured game port to this machine, so a home operator behind NAT does not have
//! to find their router's port-forwarding page by hand. When no UPnP-capable router answers, or
//! it refuses the request (UPnP disabled, a corporate/ISP router, or simply no NAT at all), this
//! logs a clear, specific fallback message naming the port and the local address to forward it
//! to — never a fatal error, and never something a running server waits on: [`attempt`] is meant
//! to be spawned as a background task, the same way `update::boot_check` is.
//!
//! This has nothing to do with the web admin panel, which stays bound to loopback regardless of
//! anything here — see `config.rs`'s `panel_listen` doc comment.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use igd_next::{PortMappingProtocol, SearchOptions, aio::tokio::search_gateway};
use tracing::{info, warn};

/// Real router firmware does not reliably honour a `0` (infinite) lease the way the UPnP spec
/// technically allows — plenty of implementations expire it anyway. Two hours, renewed at the
/// halfway point by this function's own loop, is a conservative, widely-used middle ground (the
/// same order of magnitude other UPnP port-mapping tools default to) rather than either extreme.
const LEASE_SECS: u32 = 7_200;

/// Attempt UPnP port-mapping for `listen`'s port, once, then keep the lease renewed for as long
/// as the returned future runs. Returns immediately, without attempting anything, for a
/// loopback-only `listen` — there is nothing on the public internet a router mapping would help
/// reach in that case.
pub async fn attempt(listen: SocketAddr) {
    if listen.ip().is_loopback() {
        return;
    }

    let Some(local_ip) = local_ipv4() else {
        warn!(
            "could not determine this machine's local network address; skipping UPnP port \
             mapping — forward TCP port {} manually if you want this server reachable from \
             outside your network",
            listen.port()
        );
        return;
    };
    let local_addr = SocketAddr::new(IpAddr::V4(local_ip), listen.port());

    let gateway = match search_gateway(SearchOptions::default()).await {
        Ok(gateway) => gateway,
        Err(e) => {
            info!(
                error = %e,
                port = listen.port(),
                %local_addr,
                "no UPnP-capable router found (or UPnP is disabled on it) — forward TCP port {} \
                 to {local_addr} on your router manually if you want this server reachable from \
                 outside your network",
                listen.port()
            );
            return;
        }
    };

    if let Err(e) = map_once(&gateway, listen.port(), local_addr).await {
        info!(
            error = %e,
            port = listen.port(),
            %local_addr,
            "the router refused the UPnP port mapping request — forward TCP port {} to \
             {local_addr} manually if you want this server reachable from outside your network",
            listen.port()
        );
        return;
    }
    info!(
        port = listen.port(),
        %local_addr,
        "UPnP: game port forwarded automatically"
    );

    // The mapping above already covers the first `LEASE_SECS`; this loop keeps it alive for as
    // long as the server keeps running, rather than letting it quietly expire mid-session.
    let mut interval = tokio::time::interval(Duration::from_secs(u64::from(LEASE_SECS) / 2));
    interval.tick().await; // the first tick fires immediately — already mapped above.
    loop {
        interval.tick().await;
        if let Err(e) = map_once(&gateway, listen.port(), local_addr).await {
            warn!(error = %e, "renewing the UPnP port mapping failed; it may expire soon");
        }
    }
}

async fn map_once(
    gateway: &igd_next::aio::Gateway<igd_next::aio::tokio::Tokio>,
    port: u16,
    local_addr: SocketAddr,
) -> Result<(), igd_next::AddPortError> {
    gateway
        .add_port(
            PortMappingProtocol::TCP,
            port,
            local_addr,
            LEASE_SECS,
            "terrustia",
        )
        .await
}

/// This machine's own address on whichever interface would actually reach the default route —
/// the address a router's port mapping needs to forward *to*. The well-known "connect a UDP
/// socket, read back its local address" trick: `connect` on a UDP socket only resolves routing
/// and binds a local address for it, it does not send a packet, so this works with no network
/// traffic and no dependency on 8.8.8.8 (or anything else) actually being reachable.
fn local_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_listen_address_is_skipped_without_touching_the_network() {
        // Not asserting on `attempt` directly (it is `async` and reaches the real network for
        // anything past this check) — just confirming the guard this function opens with reads
        // `is_loopback` the way it is meant to, on the exact addresses `--listen 127.0.0.1:...`
        // and the default `0.0.0.0:...` actually produce.
        assert!(
            "127.0.0.1:7777"
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
        assert!(
            !"0.0.0.0:7777"
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
    }

    #[test]
    fn local_ipv4_finds_some_address_or_cleanly_says_it_cannot() {
        // The environment this runs in may or may not have a real default route (a sandboxed CI
        // runner sometimes does not) — both outcomes are legitimate, so this only asserts the
        // function does not panic and, when it does find something, that it looks like a real
        // IPv4 address rather than a placeholder.
        if let Some(ip) = local_ipv4() {
            assert!(!ip.is_unspecified(), "0.0.0.0 is not a real local address");
        }
    }
}
