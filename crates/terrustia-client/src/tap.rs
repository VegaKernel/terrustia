//! Record every byte a connection carries, in both directions.
//!
//! The server has had `--record` for a while, which captures what a *real client* sends. This is
//! the mirror of it, and it closes the more valuable half of the same gap: pointed at a real
//! `TerrariaServer`, it captures bytes that Re-Logic's code produced. Those owe nothing to
//! `terrustia-proto`, so anything decoded out of them is checked rather than merely self-consistent.
//!
//! The file format is the server's `TRCAP1`, byte for byte, so one reader serves both:
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
//! It is written here rather than imported because `terrustia-client` does not depend on the
//! server crate, and giving it one so a capture could be written would be a heavy price for a
//! header and two integers. The format is pinned by [`tests::the_format_matches_the_server`].

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
    time::Instant,
};

/// The file's first bytes, identical to the server's recorder.
pub const MAGIC: &[u8] = b"TRCAP1\n";

/// Which way a chunk was travelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From this client to the server.
    ToServer,
    /// From the server to this client. The interesting direction when the far end is real
    /// Terraria: these bytes were produced by the game itself.
    ToClient,
}

impl Direction {
    fn as_byte(self) -> u8 {
        match self {
            // Matches the server's recorder, where 0 is "produced by the far end of the socket
            // from the recorder". Here the recorder is the client, so the sense is flipped and the
            // byte is chosen to keep the *file* consistent: 0 is always client-to-server.
            Self::ToServer => 0,
            Self::ToClient => 1,
        }
    }
}

/// An open capture file.
pub struct Tap {
    out: BufWriter<File>,
    started: Instant,
}

impl Tap {
    /// Create a capture, replacing anything already at that path.
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(MAGIC)?;
        Ok(Self {
            out,
            started: Instant::now(),
        })
    }

    /// Record one socket read or write, exactly as it crossed the wire.
    ///
    /// Errors are dropped rather than propagated: a capture is a diagnostic, and failing a
    /// connection because the observation of it could not be written would make the tool the
    /// problem it was added to find.
    pub fn chunk(&mut self, direction: Direction, bytes: &[u8]) {
        let micros = self.started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
        let mut header = [0u8; 10];
        header[0] = direction.as_byte();
        header[1] = 0; // slot; a client capture only ever holds the one connection
        header[2..6].copy_from_slice(&micros.to_le_bytes());
        header[6..10].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        let _ = self.out.write_all(&header);
        let _ = self.out.write_all(bytes);
    }

    /// Flush what is buffered, so a capture read while the client is still running is complete
    /// up to this point.
    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header must stay exactly the shape the server's reader expects, since the whole point
    /// is that one reader serves both. Checked by construction rather than by importing the
    /// server crate, which the client deliberately does not depend on.
    #[test]
    fn the_format_matches_the_server() {
        let dir = std::env::temp_dir().join(format!("tap-format-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.trcap");
        {
            let mut tap = Tap::create(&path).unwrap();
            tap.chunk(Direction::ToServer, &[1, 2, 3]);
            tap.chunk(Direction::ToClient, &[9]);
        }
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..MAGIC.len()], MAGIC);

        let mut at = MAGIC.len();
        // First chunk: outbound, slot 0, three bytes.
        assert_eq!(raw[at], 0, "client-to-server is direction 0");
        assert_eq!(raw[at + 1], 0, "slot");
        assert_eq!(
            u32::from_le_bytes(raw[at + 6..at + 10].try_into().unwrap()),
            3
        );
        assert_eq!(&raw[at + 10..at + 13], &[1, 2, 3]);
        at += 13;

        assert_eq!(raw[at], 1, "server-to-client is direction 1");
        assert_eq!(
            u32::from_le_bytes(raw[at + 6..at + 10].try_into().unwrap()),
            1
        );
        assert_eq!(raw[at + 10], 9);
        assert_eq!(at + 11, raw.len(), "nothing trailing");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_capture_of_nothing_is_still_a_valid_file() {
        let dir = std::env::temp_dir().join(format!("tap-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.trcap");
        drop(Tap::create(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), MAGIC);
        std::fs::remove_dir_all(&dir).ok();
    }
}
