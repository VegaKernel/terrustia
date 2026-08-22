//! Throw malformed packets at a running server and see whether it survives.
//!
//! A server is reachable by anyone, so every packet it takes is untrusted input. The unit tests
//! check that well-formed packets do the right thing; this checks that badly-formed ones do
//! nothing at all — no panic, no hang, no crash — which is a different property and one that only
//! shows up under real traffic.
//!
//! It joins properly first, so the fuzzing reaches the handlers that require a live player rather
//! than bouncing off the handshake.
//!
//! ```sh
//! cargo run --release -- --world some.wld &
//! cargo run --release --example fuzz -- 127.0.0.1:7777 20000
//! ```

use std::{env, process::ExitCode, time::Duration};

use rand::{Rng, SeedableRng, rngs::SmallRng};
use terrustia_client::Client;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let rounds: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20_000);
    let Ok(addr) = addr.parse() else {
        eprintln!("usage: fuzz [host:port] [rounds]");
        return ExitCode::FAILURE;
    };

    let mut client = match Client::join(addr, "fuzz").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not join {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_secs(5));

    let mut rng = SmallRng::seed_from_u64(0xF0FF);
    let mut sent = 0usize;
    for round in 0..rounds {
        // Half of it is random noise; the other half is structurally plausible traffic for the
        // handlers that take coordinates, with the coordinates pushed to their extremes. That is
        // where a bounds bug actually lives — a handler is far likelier to mishandle a real-looking
        // packet naming tile (-32768, 32767) than a payload of noise it rejects at the first field.
        let (id, mut payload) = if rng.random_bool(0.5) {
            let len = rng.random_range(0..64usize);
            let mut payload = vec![0u8; len];
            rng.fill(&mut payload[..]);
            (rng.random::<u8>(), payload)
        } else {
            // Every packet here begins with a tile coordinate pair.
            let takes_coords = [17u8, 20, 34, 48, 52, 59, 63, 64, 87, 105, 113];
            let id = takes_coords[rng.random_range(0..takes_coords.len())];
            let edge = [i16::MIN, -1, 0, 1, i16::MAX, 4199, 4200, 1199, 1200];
            let x = edge[rng.random_range(0..edge.len())];
            let y = edge[rng.random_range(0..edge.len())];
            let mut payload = Vec::new();
            // A leading action byte for the ones that have one.
            if matches!(id, 17 | 34 | 52) {
                payload.push(rng.random());
            }
            payload.extend_from_slice(&x.to_le_bytes());
            payload.extend_from_slice(&y.to_le_bytes());
            let tail = rng.random_range(0..12usize);
            let mut rest = vec![0u8; tail];
            rng.fill(&mut rest[..]);
            payload.extend_from_slice(&rest);
            (id, payload)
        };
        let len = payload.len();
        payload.truncate(len);
        let mut frame = Vec::with_capacity(len + 3);
        frame.extend_from_slice(&((len + 3) as u16).to_le_bytes());
        frame.push(id);
        frame.extend_from_slice(&payload);

        if client.send(&frame).await.is_err() {
            eprintln!("the server stopped taking packets after {sent} of {rounds}");
            return ExitCode::FAILURE;
        }
        sent += 1;

        // Every so often, check it is still answering rather than merely still connected.
        if round % 2_000 == 1_999 {
            client.say("/players").await.ok();
            let alive = client
                .try_wait_for(
                    "an answer",
                    |e| matches!(e, terrustia_client::Event::Chat { .. }),
                    Duration::from_secs(3),
                )
                .await;
            if alive.is_none() {
                eprintln!("the server stopped answering after {sent} packets");
                return ExitCode::FAILURE;
            }
            println!("  {sent} packets in, still answering");
        }
    }

    println!("{sent} malformed packets sent; the server is still up and answering");
    ExitCode::SUCCESS
}
