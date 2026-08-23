use std::{net::SocketAddr, time::Duration};

use bytes::{Bytes, BytesMut};
use terrustia_proto::{NetworkText, packets};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_util::codec::Decoder;
use tracing::{debug, warn};

use crate::{
    game::ServerEvent,
    net::codec::{CodecError, TerrariaCodec},
};

/// Frames queued for one client before it is considered too slow to keep.
///
/// The initial world burst is around forty section packets, so this leaves a wide margin while
/// still bounding memory for a client that has stopped reading.
const OUTBOUND_QUEUE: usize = 512;

/// Starting read buffer. Section packets are large, so a bigger buffer avoids repeated growth.
const READ_BUFFER: usize = 16 * 1024;

/// Serve one accepted connection until it closes.
pub async fn serve(
    stream: TcpStream,
    addr: SocketAddr,
    events: mpsc::Sender<ServerEvent>,
    idle_timeout: Duration,
) {
    // Terraria is latency-sensitive and its packets are small; batching them hurts responsiveness.
    if let Err(e) = stream.set_nodelay(true) {
        debug!(%addr, error = %e, "could not disable Nagle");
    }

    let (out_tx, out_rx) = mpsc::channel::<Bytes>(OUTBOUND_QUEUE);
    let (slot_tx, slot_rx) = oneshot::channel();

    if events
        .send(ServerEvent::Join {
            addr,
            out: out_tx,
            slot: slot_tx,
        })
        .await
        .is_err()
    {
        return; // the game task is gone; nothing to serve
    }

    let (mut read_half, mut write_half) = stream.into_split();

    let slot = match slot_rx.await {
        Ok(Some(slot)) => slot,
        _ => {
            // Full, or the game task dropped the request. Say so rather than closing silently:
            // a bare disconnect shows up in the client as an unexplained failure.
            if let Ok(frame) = packets::kick(&NetworkText::literal("This server is full.")) {
                let _ = write_half.write_all(&frame).await;
                let _ = write_half.flush().await;
            }
            return;
        }
    };

    let writer = tokio::spawn(write_loop(out_rx, write_half));
    let reason = read_loop(&mut read_half, slot, &events, idle_timeout).await;
    debug!(%addr, slot, %reason, "connection closed");

    // Dropping the player closes the outbound channel, which ends the write task.
    let _ = events.send(ServerEvent::Leave { slot }).await;
    writer.abort();
}

/// Pump the outbound queue to the socket.
///
/// Frames arrive fully encoded, so this is a plain write rather than a `FramedWrite`.
/// How much of the queue one write may coalesce.
///
/// Frames are small and there are a lot of them: with players in sight of one another, every
/// movement one makes is a frame to each of the others. Writing them one at a time is one syscall
/// each, and the queue then drains slower than the game fills it — which shows up not as slowness
/// but as clients being *dropped* for falling behind. Gathering whatever is already waiting into
/// one write costs nothing and is what stops that.
const WRITE_BATCH: usize = 64 * 1024;

async fn write_loop(mut out: mpsc::Receiver<Bytes>, mut sink: tokio::net::tcp::OwnedWriteHalf) {
    let mut batch: Vec<u8> = Vec::with_capacity(WRITE_BATCH);
    while let Some(frame) = out.recv().await {
        batch.clear();
        batch.extend_from_slice(&frame);
        // Everything else already queued goes out in the same write. `try_recv` never waits, so
        // this only ever gathers what the game task has *already* produced.
        while batch.len() < WRITE_BATCH {
            match out.try_recv() {
                Ok(next) => batch.extend_from_slice(&next),
                Err(_) => break,
            }
        }
        if sink.write_all(&batch).await.is_err() {
            break;
        }
    }
    let _ = sink.shutdown().await;
}

/// Decode frames off the socket and forward them to the game task.
async fn read_loop(
    read: &mut tokio::net::tcp::OwnedReadHalf,
    slot: u8,
    events: &mpsc::Sender<ServerEvent>,
    idle_timeout: Duration,
) -> &'static str {
    let mut codec = TerrariaCodec;
    let mut buf = BytesMut::with_capacity(READ_BUFFER);

    loop {
        // Drain everything already buffered before waiting on the socket again.
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    if events
                        .send(ServerEvent::Packet { slot, frame })
                        .await
                        .is_err()
                    {
                        return "server shutting down";
                    }
                }
                Ok(None) => break,
                Err(CodecError::FrameTooShort { len }) => {
                    warn!(slot, len, "desynchronised stream");
                    return "protocol error";
                }
                Err(e) => {
                    warn!(slot, error = %e, "decode failed");
                    return "protocol error";
                }
            }
        }

        match timeout(idle_timeout, read.read_buf(&mut buf)).await {
            Ok(Ok(0)) => return "client closed",
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                debug!(slot, error = %e, "socket read failed");
                return "read error";
            }
            Err(_) => return "idle timeout",
        }
    }
}
