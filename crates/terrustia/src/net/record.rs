//! Record the raw bytes of a connection, in both directions, to a file.
//!
//! Every automated test in this repository has the same blind spot: `terrustia-client` is built on
//! `terrustia-proto`, the very crate the server encodes with. If a field is read at the wrong
//! width, both sides read it at the wrong width, the two agree perfectly, and every test passes.
//! No amount of testing against our own client can find that class of bug, because the bug is in
//! the shared assumption rather than in either side of it.
//!
//! A real Terraria client's bytes are not derived from this code, so they are the one independent
//! opinion available. This captures them: run the server with `--record`, connect the real game,
//! play for a few minutes, and the file that falls out is a corpus that can be replayed
//! afterwards — and checked into the repository, so the answer stays checked rather than being a
//! thing somebody once did.
//!
//! The format is deliberately trivial, so that reading it needs nothing this crate does not
//! already depend on:
//!
//! ```text
//! "TRCAP1\n"                          magic and version
//! repeated:
//!   u8   direction   0 = client to server, 1 = server to client
//!   u8   slot
//!   u32  microseconds since the recording started
//!   u32  length
//!   ..   that many bytes, exactly as they crossed the socket
//! ```
//!
//! Chunks are socket reads and writes, not frames: a chunk may hold several frames or half of
//! one. Re-framing is the reader's job, which is the point — it means the capture can prove the
//! framing itself, not just the payloads.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use tokio::sync::mpsc;
use tracing::{error, info};

/// The file's first bytes, so a truncated or unrelated file is rejected rather than misread.
pub const MAGIC: &[u8] = b"TRCAP1\n";

/// Which way a chunk was travelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the real client to this server. The interesting direction: these bytes were produced
    /// by Terraria itself.
    Inbound,
    /// From this server to the client.
    Outbound,
}

impl Direction {
    fn as_byte(self) -> u8 {
        match self {
            Self::Inbound => 0,
            Self::Outbound => 1,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Inbound),
            1 => Some(Self::Outbound),
            _ => None,
        }
    }
}

/// One recorded chunk of socket traffic.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub direction: Direction,
    pub slot: u8,
    pub micros: u32,
    pub bytes: Vec<u8>,
}

/// A handle the connection tasks push bytes into.
///
/// Cheap to clone — every connection gets one. Sends are unbounded and never block: a recorder
/// that applied backpressure to the network path would change the very timing it is meant to
/// observe, and a capture is a debugging aid rather than something worth stalling a game for.
#[derive(Debug, Clone)]
pub struct Recorder {
    tx: mpsc::UnboundedSender<Chunk>,
    started: Instant,
}

impl Recorder {
    /// Start recording to `path`, spawning the task that owns the file.
    pub fn create(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut file = std::fs::File::create(&path)?;
        file.write_all(MAGIC)?;

        let (tx, mut rx) = mpsc::unbounded_channel::<Chunk>();
        info!(path = %path.display(), "recording connections");

        tokio::spawn(async move {
            let mut written = 0u64;
            while let Some(chunk) = rx.recv().await {
                let mut header = [0u8; 10];
                header[0] = chunk.direction.as_byte();
                header[1] = chunk.slot;
                header[2..6].copy_from_slice(&chunk.micros.to_le_bytes());
                header[6..10].copy_from_slice(&(chunk.bytes.len() as u32).to_le_bytes());
                if let Err(e) = file
                    .write_all(&header)
                    .and_then(|()| file.write_all(&chunk.bytes))
                {
                    error!(error = %e, "could not write to the capture; recording stops here");
                    return;
                }
                written += chunk.bytes.len() as u64;
            }
            // The channel closes when the last connection goes, which is the end of the capture.
            if let Err(e) = file.flush() {
                error!(error = %e, "could not flush the capture");
            }
            info!(path = %path.display(), bytes = written, "capture closed");
        });

        Ok(Self {
            tx,
            started: Instant::now(),
        })
    }

    /// Record one chunk. Does nothing once the writer task has gone.
    pub fn chunk(&self, direction: Direction, slot: u8, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = self.tx.send(Chunk {
            direction,
            slot,
            micros: self.started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32,
            bytes: bytes.to_vec(),
        });
    }
}

/// Read a capture file back into its chunks.
///
/// Returns an error for anything that is not a capture, and stops cleanly at the first truncated
/// record: a capture whose server was killed mid-write is still worth everything before the cut.
pub fn read(path: &Path) -> io::Result<Vec<Chunk>> {
    let bytes = std::fs::read(path)?;
    if !bytes.starts_with(MAGIC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a terrustia capture", path.display()),
        ));
    }

    let mut chunks = Vec::new();
    let mut at = MAGIC.len();
    while at + 10 <= bytes.len() {
        let Some(direction) = Direction::from_byte(bytes[at]) else {
            break;
        };
        let slot = bytes[at + 1];
        let micros = u32::from_le_bytes(bytes[at + 2..at + 6].try_into().expect("4 bytes"));
        let len = u32::from_le_bytes(bytes[at + 6..at + 10].try_into().expect("4 bytes")) as usize;
        at += 10;
        if at + len > bytes.len() {
            break; // truncated tail
        }
        chunks.push(Chunk {
            direction,
            slot,
            micros,
            bytes: bytes[at..at + len].to_vec(),
        });
        at += len;
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_round_trips() {
        let dir = std::env::temp_dir().join(format!("terrustia-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("round-trip.trcap");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let recorder = Recorder::create(&path).expect("create");
            recorder.chunk(Direction::Inbound, 3, &[1, 2, 3]);
            recorder.chunk(Direction::Outbound, 3, &[4, 5]);
            // Dropping the last handle closes the channel, which is what ends the writer task.
            drop(recorder);
            // Let the writer task run to completion before the file is read back.
            tokio::task::yield_now().await;
            for _ in 0..100 {
                if read(&path).is_ok_and(|c| c.len() == 2) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });

        let chunks = read(&path).expect("read back");
        assert_eq!(chunks.len(), 2, "both chunks should be there");
        assert_eq!(chunks[0].direction, Direction::Inbound);
        assert_eq!(chunks[0].slot, 3);
        assert_eq!(chunks[0].bytes, vec![1, 2, 3]);
        assert_eq!(chunks[1].direction, Direction::Outbound);
        assert_eq!(chunks[1].bytes, vec![4, 5]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_capture_is_refused() {
        let dir = std::env::temp_dir().join(format!("terrustia-nocap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("not-a-capture");
        std::fs::write(&path, b"hello").expect("write");
        assert!(read(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
