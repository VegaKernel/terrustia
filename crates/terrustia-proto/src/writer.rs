use std::ops::{Deref, DerefMut};

use crate::error::{ProtoError, Result};

/// A frame's 2-byte length prefix covers the whole frame, itself included.
pub const MAX_FRAME_LEN: usize = u16::MAX as usize;

/// Growable byte sink with .NET `BinaryWriter` semantics: little-endian numbers and strings
/// prefixed by a 7-bit-encoded byte count.
///
/// Used directly for unframed streams — the inner tile-section stream is built with one of these
/// before being deflated — and wrapped by [`PacketWriter`] for anything that goes on the wire.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

macro_rules! write_le {
    ($($(#[$m:meta])* $name:ident($ty:ty)),* $(,)?) => {$(
        $(#[$m])*
        pub fn $name(&mut self, value: $ty) -> &mut Self {
            self.buf.extend_from_slice(&value.to_le_bytes());
            self
        }
    )*};
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    write_le! {
        u8(u8),
        i8(i8),
        u16(u16),
        i16(i16),
        u32(u32),
        i32(i32),
        u64(u64),
        i64(i64),
        f32(f32),
        f64(f64),
    }

    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.u8(u8::from(value))
    }

    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(value);
        self
    }

    /// `BinaryWriter.Write7BitEncodedInt`: seven bits per byte, little end first.
    pub fn var_u32(&mut self, mut value: u32) -> &mut Self {
        while value >= 0x80 {
            self.buf.push((value as u8) | 0x80);
            value >>= 7;
        }
        self.buf.push(value as u8);
        self
    }

    /// `BinaryWriter.Write(string)`: 7-bit-encoded UTF-8 *byte* count, then the bytes.
    pub fn string(&mut self, value: &str) -> &mut Self {
        self.var_u32(value.len() as u32);
        self.bytes(value.as_bytes())
    }

    /// Three bytes; Terraria never sends an alpha channel over the network.
    pub fn rgb(&mut self, value: [u8; 3]) -> &mut Self {
        self.bytes(&value)
    }

    /// An XNA `Vector2`: two little-endian floats.
    pub fn vec2(&mut self, x: f32, y: f32) -> &mut Self {
        self.f32(x).f32(y)
    }
}

/// A [`Writer`] that already holds a reserved length prefix and a message id, so it can only be
/// finished into a complete frame.
#[derive(Debug, Clone)]
pub struct PacketWriter {
    inner: Writer,
}

impl PacketWriter {
    /// Start a frame: two zero bytes standing in for the length, then the message id.
    pub fn new(id: u8) -> Self {
        let mut inner = Writer::with_capacity(64);
        inner.u16(0).u8(id);
        Self { inner }
    }

    pub fn message_id(&self) -> u8 {
        self.inner.as_slice()[2]
    }

    /// Patch the reserved prefix with the final length and hand back the frame.
    pub fn finish(self) -> Result<Vec<u8>> {
        let mut buf = self.inner.into_bytes();
        let len = buf.len();
        if len > MAX_FRAME_LEN {
            return Err(ProtoError::FrameTooLarge {
                len,
                max: MAX_FRAME_LEN,
            });
        }
        buf[..2].copy_from_slice(&(len as u16).to_le_bytes());
        Ok(buf)
    }
}

impl Deref for PacketWriter {
    type Target = Writer;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for PacketWriter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::PacketReader;

    #[test]
    fn frame_length_includes_its_own_prefix_and_the_id() {
        let mut w = PacketWriter::new(0x2A);
        w.i32(1).u8(2);
        let frame = w.finish().unwrap();

        // 2 length + 1 id + 4 + 1 payload
        assert_eq!(frame.len(), 8);
        assert_eq!(&frame[..3], &[8, 0, 0x2A]);
        assert_eq!(
            u16::from_le_bytes([frame[0], frame[1]]) as usize,
            frame.len()
        );
    }

    #[test]
    fn empty_payload_frame_is_three_bytes() {
        let frame = PacketWriter::new(6).finish().unwrap();
        assert_eq!(frame, vec![3, 0, 6]);
    }

    #[test]
    fn oversized_frame_is_rejected_rather_than_truncated() {
        let mut w = PacketWriter::new(10);
        w.bytes(&vec![0u8; MAX_FRAME_LEN]);
        assert!(matches!(
            w.finish(),
            Err(ProtoError::FrameTooLarge {
                max: MAX_FRAME_LEN,
                ..
            })
        ));
    }

    #[test]
    fn var_u32_round_trips_through_the_reader() {
        for value in [0, 1, 127, 128, 300, 16_383, 16_384, u32::MAX] {
            let mut w = Writer::new();
            w.var_u32(value);
            let bytes = w.into_bytes();
            assert_eq!(PacketReader::new(&bytes).var_u32().unwrap(), value);
        }
    }

    #[test]
    fn primitives_round_trip_through_the_reader() {
        let mut w = Writer::new();
        w.i16(-2)
            .u32(0xDEAD_BEEF)
            .f32(1.5)
            .bool(true)
            .string("hello \u{1F600}")
            .rgb([1, 2, 3])
            .vec2(-0.25, 64.0);
        let bytes = w.into_bytes();

        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.i16().unwrap(), -2);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.f32().unwrap(), 1.5);
        assert!(r.bool().unwrap());
        assert_eq!(r.string().unwrap(), "hello \u{1F600}");
        assert_eq!(r.rgb().unwrap(), [1, 2, 3]);
        assert_eq!(r.vec2().unwrap(), (-0.25, 64.0));
        assert!(r.is_empty());
    }

    #[test]
    fn long_strings_round_trip() {
        let text = "a".repeat(5000);
        let mut w = Writer::new();
        w.string(&text);
        let bytes = w.into_bytes();
        assert_eq!(PacketReader::new(&bytes).string().unwrap(), text);
    }
}
