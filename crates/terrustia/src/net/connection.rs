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
    net::record,
};

/// Frames queued for one client before it is considered too slow to keep, ignoring other players.
///
/// The burst a joining client is sent is much larger than it looks. Around forty section packets,
/// yes — but also one frame per item on the ground (up to 400), one per live NPC (up to 200), and
/// the world and progress packets. A flat 512 covered the sections and nothing else.
///
/// Chest contents are what makes it big. Every section carries the full inventory of every chest
/// inside it — forty frames per chest, as the game sends them — so one storage room in view of
/// spawn is worth more frames than the entire rest of the join put together. Fifty chests in a
/// section is two thousand frames from that section alone, and a player dropped for a full queue
/// is dropped silently, mid-load, with no message. The cost of being generous here is a queue of
/// pointers that is only ever that deep for the second or two a join takes.
const OUTBOUND_BASE: usize = 8192;

/// Extra room per player slot the server is configured for.
///
/// This constant used to be 256, sized against a theory that turned out to be only half right:
/// "every other player already in the world costs the newcomer their presence frames and one
/// relayed inventory slot each, ~200 frames" — a one-off *join-time* cost, paid once per newcomer.
/// A synchronized 255-player join (`examples/crowd`, the same shape `docs/performance.md`'s "Large
/// world, 16 and 255 players" section measured) dropped roughly half the crowd within 5-30 seconds
/// of joining even though that theory's own numbers fit comfortably inside the old queue. Reading
/// the drop log's own `packet`/`name` fields — which were sitting right there and unchecked —
/// shows why: the drops are almost entirely `PlayerControls` (movement) and a handful of
/// `SyncNPC`, and the dropped slots spread uniformly across the whole range rather than clustering
/// among the earliest joiners the way an unread join-time backlog would.
///
/// The real mechanism is `on_player_controls`'s unconditional broadcast in `game/server.rs`: once
/// everyone is moving, every one of up to `max_players - 1` peers relays a control packet to every
/// other peer roughly once a tick — genuinely O(n²) in player count, and nothing to do with *how*
/// the population got there, only how big it is. At `max_players = 255` that is up to 254 frames
/// landing in one already-established client's queue every ~16ms the moment the server's own
/// scheduling falls behind for even a few seconds — a loaded machine, a burst of `SyncNPC` while
/// wildlife spawns in — which 256 per player could not absorb at all. 4,096 survived every
/// `examples/crowd` trial at this player count (including a full 90-second run under real,
/// independently-confirmed machine contention, worst case 5 drops out of 255 at a quarter of this
/// number) with tick cost unaffected — this is a queue-sizing constant, not tick-cost work, and
/// `docs/performance.md`'s own re-measurement after this change confirms that directly rather than
/// assuming it. `OUTBOUND_BASE` above was not touched: no join-time packet (a section, a chest
/// slot, an item, a newcomer's own NPC burst) ever showed up in a drop log across any trial, so it
/// was never what overflowed here.
///
/// This is a mitigation, not a cure. The O(n²) relay cost is real and is vanilla's own behaviour —
/// this project transcribes it rather than inventing a throttle vanilla does not have — so a
/// deeper queue buys headroom without removing the underlying cost, and a big enough contention
/// spike could still in principle outrun any finite number. A genuine fix would look like NPC
/// sync's own answer to the same shape of problem (skip a client whose loaded sections cannot see
/// the thing being synced) applied to `on_player_controls`'s broadcast instead of widening this
/// queue further — but that is a change to `game/server.rs`'s broadcast logic, out of this file's
/// scope, and left for whoever owns that file next.
///
/// **That fix now exists** (`GameServer::broadcast_near`, applied to player movement and client
/// projectile syncs), and it was measured against this constant rather than assumed. Two 255-player
/// runs at the old 256, differing only in whether the cull was wired up, at matched NPC load:
///
/// | | no cull | cull |
/// |---|---|---|
/// | `outbound queue full` drops | 14 | 0 |
/// | clients held | 245/255 | 255/255 |
/// | peak queue depth | 73,465 of 73,472 | 38,713 |
///
/// So the cull does what this comment predicted: without it the queue runs literally full at the
/// pre-mitigation depth, and with it the peak halves and nothing is dropped.
///
/// **4,096 stays anyway.** A separate 255-player half-hour at this depth peaked at 578,447 queued
/// frames, eight times what 256 can hold, because queue depth is what absorbs a contention spike on
/// the test box: the game loop is descheduled, the backlog builds, and it drains again afterwards.
/// Shrinking the queue would convert that recoverable backlog into dropped players. The cull
/// removes the steady-state O(n²) term; the depth still covers the transient, and the two are not
/// substitutes for each other.
const OUTBOUND_PER_PLAYER: usize = 4096;

/// The queue depth to give one connection on a server configured for `max_players`.
pub fn outbound_queue(max_players: usize) -> usize {
    OUTBOUND_BASE + max_players * OUTBOUND_PER_PLAYER
}

/// Starting read buffer. Section packets are large, so a bigger buffer avoids repeated growth.
const READ_BUFFER: usize = 16 * 1024;

/// How many frames a connection may send before it counts as having handshaked.
///
/// The real sequence is a version, a UUID, a player, an inventory and a spawn request — a couple
/// of dozen frames with the inventory slots. Past that it is an ordinary session and the idle
/// timeout is the right rule; before it, the connection is on a deadline.
const HANDSHAKE_FRAMES: u32 = 64;

/// Serve one accepted connection until it closes.
/// The limits a connection is served under.
///
/// Grouped because they travel together and are set once from the config; passing them
/// individually made `serve` a list of bare `Duration`s that were easy to swap by accident.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// How long a single read may block before the connection is considered idle.
    pub idle: Duration,
    /// How long the whole handshake has, regardless of how often bytes trickle in.
    pub handshake: Duration,
    /// Capacity of this connection's outbound queue.
    pub outbound_queue: usize,
}

pub async fn serve(
    stream: TcpStream,
    addr: SocketAddr,
    events: mpsc::Sender<ServerEvent>,
    limits: Limits,
    recorder: Option<record::Recorder>,
    // Held for the life of the connection, releasing its place on every exit path including a
    // panic. Never read — its whole job is to be dropped.
    _slot_held: crate::net::listener::ConnectionSlot,
) {
    let idle_timeout = limits.idle;
    let outbound_queue = limits.outbound_queue;
    let handshake_deadline = std::time::Instant::now() + limits.handshake;
    // Terraria is latency-sensitive and its packets are small; batching them hurts responsiveness.
    if let Err(e) = stream.set_nodelay(true) {
        debug!(%addr, error = %e, "could not disable Nagle");
    }

    let (out_tx, out_rx) = mpsc::channel::<Bytes>(outbound_queue);
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

    // `epoch` is this connection's generation counter for `slot` (see
    // `game::server::GameServer::remove_player`'s doc comment): stamped onto every `Packet` below
    // and onto this connection's own eventual `Leave`, so the game task can tell this connection's
    // events apart from a ghost's once the slot has been recycled.
    let (slot, epoch) = match slot_rx.await {
        Ok(Some(assigned)) => assigned,
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

    let writer = tokio::spawn(write_loop(out_rx, write_half, slot, recorder.clone()));
    let reason = read_loop(
        &mut read_half,
        slot,
        epoch,
        &events,
        idle_timeout,
        handshake_deadline,
        recorder.as_ref(),
    )
    .await;
    debug!(%addr, slot, epoch, %reason, "connection closed");

    // Dropping the player closes the outbound channel, which ends the write task.
    let _ = events.send(ServerEvent::Leave { slot, epoch }).await;
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

async fn write_loop(
    mut out: mpsc::Receiver<Bytes>,
    mut sink: tokio::net::tcp::OwnedWriteHalf,
    slot: u8,
    recorder: Option<record::Recorder>,
) {
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
        // Recorded before the write, so a batch that fails to go out is still visible as the last
        // thing this server tried to say.
        if let Some(recorder) = &recorder {
            recorder.chunk(record::Direction::Outbound, slot, &batch);
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
    epoch: u32,
    events: &mpsc::Sender<ServerEvent>,
    idle_timeout: Duration,
    handshake_deadline: std::time::Instant,
    recorder: Option<&record::Recorder>,
) -> &'static str {
    let mut codec = TerrariaCodec;
    let mut buf = BytesMut::with_capacity(READ_BUFFER);
    // Cleared once the connection has said anything at all beyond the first frame. Until then it
    // is on a clock: `idle_timeout` wraps each individual *read*, so its timer resets on any byte
    // and a connection trickling one byte a minute would hold its place for ever.
    let mut still_handshaking = true;
    let mut frames_seen = 0u32;

    loop {
        // Drain everything already buffered before waiting on the socket again.
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    // A handful of frames is the whole handshake; past that it is a real session
                    // and the ordinary idle timeout is the right rule.
                    frames_seen = frames_seen.saturating_add(1);
                    if frames_seen > HANDSHAKE_FRAMES {
                        still_handshaking = false;
                    }
                    if events
                        .send(ServerEvent::Packet { slot, epoch, frame })
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

        // Whichever runs out first: the per-read idle timer, or the one-shot deadline for
        // getting through the handshake at all.
        let wait = if still_handshaking {
            idle_timeout.min(
                handshake_deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .max(Duration::from_millis(1)),
            )
        } else {
            idle_timeout
        };
        if still_handshaking && std::time::Instant::now() >= handshake_deadline {
            return "took too long to say who it was";
        }

        let before = buf.len();
        match timeout(wait, read.read_buf(&mut buf)).await {
            Ok(Ok(0)) => return "client closed",
            Ok(Ok(_)) => {
                // Exactly the bytes that arrived, before anything here has looked at them. This
                // is the whole value of a capture: ground truth that owes nothing to our own
                // idea of what the client should have sent.
                if let Some(recorder) = recorder {
                    recorder.chunk(record::Direction::Inbound, slot, &buf[before..]);
                }
            }
            Ok(Err(e)) => {
                debug!(slot, error = %e, "socket read failed");
                return "read error";
            }
            Err(_) if still_handshaking && std::time::Instant::now() >= handshake_deadline => {
                return "took too long to say who it was";
            }
            Err(_) if still_handshaking => continue,
            Err(_) => return "idle timeout",
        }
    }
}
