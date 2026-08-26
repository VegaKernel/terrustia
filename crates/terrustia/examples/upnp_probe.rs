//! Manually exercise UPnP port mapping against whatever network this machine is actually on,
//! without starting a real server. Useful both as an operator-facing diagnostic ("will this
//! work on my network?") and as this feature's own real-network verification — a router-less
//! sandbox exercises the fallback path for real; a real home network exercises the mapping path
//! for real. Either outcome is a legitimate, observable result, not a failure of the tool.
//!
//! ```sh
//! cargo run --example upnp_probe -- 0.0.0.0:7777
//! ```

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:7777".to_string());
    let addr: std::net::SocketAddr = addr.parse().expect("usage: upnp_probe [HOST:PORT]");
    println!("attempting UPnP port mapping for {addr}...");
    // `attempt` never returns on success (it holds the lease open, renewing it) — a bounded
    // window is enough to observe either the mapping succeeding or the fallback message.
    tokio::select! {
        () = terrustia::upnp::attempt(addr) => {
            println!("attempt() returned early — see the fallback log line above for why");
        }
        () = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
            println!(
                "15s elapsed with no early return — if a \"forwarded automatically\" log line \
                 printed above, the mapping worked and this probe is now just holding the lease \
                 open; ctrl-c to stop"
            );
        }
    }
}
