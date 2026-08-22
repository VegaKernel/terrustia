use std::time::Duration;

use tokio::{net::TcpListener, sync::mpsc};
use tracing::{debug, error, info};

use crate::{config::Config, game::ServerEvent, net::connection};

/// Accept connections until the listener fails or the game task goes away.
pub async fn run(listener: TcpListener, config: Config, events: mpsc::Sender<ServerEvent>) {
    let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
    match listener.local_addr() {
        Ok(addr) => info!(%addr, "accepting connections"),
        Err(e) => debug!(error = %e, "listener has no local address"),
    }

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                debug!(%addr, "connection accepted");
                let events = events.clone();
                tokio::spawn(connection::serve(stream, addr, events, idle_timeout));
            }
            Err(e) => {
                // A per-connection failure (out of descriptors, client vanished mid-handshake) must
                // not take the whole listener down.
                error!(error = %e, "accept failed");
                if events.is_closed() {
                    return;
                }
            }
        }
    }
}
