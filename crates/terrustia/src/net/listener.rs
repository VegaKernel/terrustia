use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{net::TcpListener, sync::mpsc};
use tracing::{debug, error, info, warn};

use crate::{config::Config, game::ServerEvent, net::connection};

/// Bind a listening socket, turning the kernel's terse refusal into a message that says what to do
/// about it. A raw `Os { code: 28 }` in a server log tells an operator nothing; the same failure
/// carrying "the OS is out of socket resources, raise the open-file limit" tells them where to look.
/// Used for both the game port and the web panel's own bind, so neither surfaces a bare errno.
pub async fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr)
        .await
        .map_err(|e| explain_bind_failure(addr, e))
}

/// Rewrite a bind failure into an explanation, keeping the original [`std::io::ErrorKind`] so a
/// caller that matches on the kind still can.
fn explain_bind_failure(addr: SocketAddr, e: std::io::Error) -> std::io::Error {
    use std::io::ErrorKind;
    let advice = match e.kind() {
        ErrorKind::AddrInUse => format!(
            "{addr} is already in use; another server is probably bound there. Stop it, or choose a \
             different address with --listen."
        ),
        ErrorKind::PermissionDenied => format!(
            "not permitted to bind {addr}; ports below 1024 need elevated privileges. Pick a port \
             at or above 1024, or grant the privilege to bind a low one."
        ),
        ErrorKind::AddrNotAvailable => format!(
            "{addr} is not an address this machine holds; bind 0.0.0.0 to listen on every \
             interface, or use an address the host actually has."
        ),
        // ENOSPC (error 28) on a bind is not the disk: the kernel has no socket or port resources
        // left to hand out, which on a busy machine means too many sockets are already open. Matched
        // on `raw_os_error` rather than an `ErrorKind`, since std does not give this one a stable
        // kind on every platform.
        _ if e.raw_os_error() == Some(28) => format!(
            "the operating system has no socket resources left to bind {addr} (error 28, \"no space \
             left on device\"): too many sockets and ports are already open across the machine, not \
             a full disk. Close other servers, or raise the open-file limit (for example \
             `ulimit -n`), then try again."
        ),
        _ => format!("could not bind {addr}: {e}"),
    };
    std::io::Error::new(e.kind(), advice)
}

/// How many sockets are open, and from where.
///
/// The accept loop used to be unconditional: every socket that connected immediately got two
/// tasks, a sixteen-kilobyte read buffer and an outbound queue, none of which required the other
/// end to have spoken a word of the protocol. Opening sockets is cheap and there was no ceiling,
/// so one machine could exhaust this one's memory and file descriptors without ever handshaking.
///
/// A count rather than a semaphore, because the per-address limit needs the breakdown anyway, and
/// a few hundred entries is nothing to walk.
#[derive(Default)]
struct OpenConnections {
    per_address: HashMap<IpAddr, usize>,
    total: usize,
}

/// Held for as long as a connection is open; releases its place when dropped.
///
/// A guard rather than a pair of calls, so a connection task that panics or returns early still
/// gives its slot back. Leaking these is exactly the failure the limit exists to prevent.
pub struct ConnectionSlot {
    open: Arc<Mutex<OpenConnections>>,
    address: IpAddr,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let mut open = match self.open.lock() {
            Ok(open) => open,
            // A poisoned lock means another task panicked while holding it. Losing the count is
            // worse than the panic, so take it anyway.
            Err(poisoned) => poisoned.into_inner(),
        };
        open.total = open.total.saturating_sub(1);
        if let Some(count) = open.per_address.get_mut(&self.address) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.per_address.remove(&self.address);
            }
        }
    }
}

/// Take a place, or say why not.
fn claim(
    open: &Arc<Mutex<OpenConnections>>,
    address: IpAddr,
    max_total: usize,
    max_per_address: usize,
) -> Result<ConnectionSlot, &'static str> {
    let mut guard = match open.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.total >= max_total {
        return Err("the server is full of connections");
    }
    let count = guard.per_address.entry(address).or_insert(0);
    if *count >= max_per_address {
        return Err("too many connections from one address");
    }
    *count += 1;
    guard.total += 1;
    Ok(ConnectionSlot {
        open: Arc::clone(open),
        address,
    })
}

/// Accept connections until the listener fails or the game task goes away.
pub async fn run(
    listener: TcpListener,
    config: Config,
    events: mpsc::Sender<ServerEvent>,
    recorder: Option<crate::net::record::Recorder>,
) {
    let limits = connection::Limits {
        idle: Duration::from_secs(config.idle_timeout_secs),
        handshake: Duration::from_secs(config.handshake_timeout_secs),
        outbound_queue: connection::outbound_queue(config.max_players),
    };
    let open = Arc::new(Mutex::new(OpenConnections::default()));
    match listener.local_addr() {
        Ok(addr) => info!(%addr, "accepting connections"),
        Err(e) => debug!(error = %e, "listener has no local address"),
    }

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let slot = match claim(
                    &open,
                    addr.ip(),
                    config.max_connections,
                    config.max_connections_per_address,
                ) {
                    Ok(slot) => slot,
                    Err(why) => {
                        // Dropped without ceremony. Anything else — a message, a graceful close —
                        // is work this server does on behalf of whoever is flooding it.
                        warn!(%addr, why, "refusing a connection");
                        drop(stream);
                        continue;
                    }
                };
                debug!(%addr, "connection accepted");
                let events = events.clone();
                tokio::spawn(connection::serve(
                    stream,
                    addr,
                    events,
                    limits,
                    recorder.clone(),
                    slot,
                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8) -> IpAddr {
        IpAddr::from([127, 0, 0, last])
    }

    #[test]
    fn the_server_stops_accepting_once_it_is_full() {
        let open = Arc::new(Mutex::new(OpenConnections::default()));
        let held: Vec<_> = (0..4)
            .map(|i| claim(&open, addr(i), 4, 4).expect("within both limits"))
            .collect();
        assert!(
            claim(&open, addr(9), 4, 4).is_err(),
            "the total is a ceiling, or one machine can open sockets until this one runs out"
        );
        drop(held);
        assert!(claim(&open, addr(9), 4, 4).is_ok(), "space frees up again");
    }

    #[test]
    fn one_address_cannot_take_every_place() {
        let open = Arc::new(Mutex::new(OpenConnections::default()));
        let _held: Vec<_> = (0..2)
            .map(|_| claim(&open, addr(1), 100, 2).expect("within the per-address limit"))
            .collect();
        assert!(
            claim(&open, addr(1), 100, 2).is_err(),
            "one address is capped even when the server has room"
        );
        assert!(
            claim(&open, addr(2), 100, 2).is_ok(),
            "and everyone else is unaffected by it"
        );
    }

    /// The guard has to release on every path, including a task that unwinds.
    #[test]
    fn a_slot_is_released_even_if_its_task_panics() {
        let open = Arc::new(Mutex::new(OpenConnections::default()));
        let taken = std::panic::catch_unwind({
            let open = Arc::clone(&open);
            move || {
                let _slot = claim(&open, addr(1), 1, 1).expect("the only place");
                panic!("a connection task falling over");
            }
        });
        assert!(taken.is_err(), "the panic should have happened");
        assert!(
            claim(&open, addr(1), 1, 1).is_ok(),
            "a panicking connection must not leak its place, or the server fills up with ghosts"
        );
    }

    /// A bind failure should name the address and say what to do, not surface a bare errno. The
    /// ENOSPC case is the one that prompted this: `os error 28` on a port bind is socket-resource
    /// exhaustion, and the message has to say so rather than let an operator chase a full disk.
    #[test]
    fn a_bind_failure_explains_itself() {
        use std::io::{Error, ErrorKind};
        let a: SocketAddr = "0.0.0.0:7777".parse().unwrap();

        let in_use = explain_bind_failure(a, Error::from(ErrorKind::AddrInUse));
        assert_eq!(in_use.kind(), ErrorKind::AddrInUse, "kind is preserved");
        assert!(in_use.to_string().contains("7777") && in_use.to_string().contains("in use"));

        let denied = explain_bind_failure(a, Error::from(ErrorKind::PermissionDenied));
        assert!(denied.to_string().contains("privilege"));

        let unavailable = explain_bind_failure(a, Error::from(ErrorKind::AddrNotAvailable));
        assert!(unavailable.to_string().contains("0.0.0.0"));

        let no_space = explain_bind_failure(a, Error::from_raw_os_error(28));
        let msg = no_space.to_string();
        assert!(
            msg.contains("error 28") && msg.contains("socket") && msg.contains("not a full disk"),
            "ENOSPC must be explained as socket exhaustion, got: {msg}"
        );

        let other = explain_bind_failure(a, Error::from(ErrorKind::ConnectionReset));
        assert!(other.to_string().contains("could not bind"));
    }
}
