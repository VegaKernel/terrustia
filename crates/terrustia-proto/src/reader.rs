use crate::error::{ProtoError, Result};

/// Cursor over a packet payload, mirroring the read side of .NET's `BinaryReader`.
///
/// Terraria's netcode is a `BinaryReader` over a `MemoryStream`, so everything here is
/// little-endian and strings carry a 7-bit-encoded *byte* length.
#[derive(Debug, Clone)]
pub struct PacketReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

macro_rules! read_le {
    ($($(#[$m:meta])* $name:ident -> $ty:ty),* $(,)?) => {$(
        $(#[$m])*
        pub fn $name(&mut self) -> Result<$ty> {
            let bytes = self.take(size_of::<$ty>())?;
            // `take` returned exactly size_of::<$ty>() bytes, so the conversion cannot fail.
            Ok(<$ty>::from_le_bytes(bytes.try_into().unwrap()))
        }
    )*};
}

impl<'a> PacketReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(ProtoError::Eof {
                offset: self.pos,
                needed: n,
                available: self.remaining(),
            });
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    read_le! {
        u8 -> u8,
        i8 -> i8,
        u16 -> u16,
        i16 -> i16,
        u32 -> u32,
        i32 -> i32,
        u64 -> u64,
        i64 -> i64,
        f32 -> f32,
        f64 -> f64,
    }

    /// A .NET `bool`: one byte, any non-zero value being true.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    /// Everything not yet consumed, leaving the cursor at the end.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = &self.buf[self.pos..];
        self.pos = self.buf.len();
        out
    }

    /// `BinaryReader.Read7BitEncodedInt`: up to five bytes, seven bits each, little end first.
    ///
    /// .NET rejects a fifth byte that still sets the continuation bit, and so do we — otherwise a
    /// hostile client could spin the loop over an arbitrarily long run of 0x80 bytes.
    pub fn var_u32(&mut self) -> Result<u32> {
        let start = self.pos;
        let mut value: u32 = 0;
        for shift in [0u32, 7, 14, 21, 28] {
            let byte = self.u8()?;
            value |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                // The final byte may not carry bits beyond the 32-bit range.
                if shift == 28 && byte > 0x0F {
                    return Err(ProtoError::BadVarInt { offset: start });
                }
                return Ok(value);
            }
        }
        Err(ProtoError::BadVarInt { offset: start })
    }

    /// `BinaryReader.ReadString`: 7-bit-encoded UTF-8 *byte* count, then that many bytes.
    pub fn string(&mut self) -> Result<String> {
        let offset = self.pos;
        let len = self.var_u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtoError::Utf8 { offset })
    }

    /// Terraria colours travel as three bytes; the alpha channel is never sent.
    pub fn rgb(&mut self) -> Result<[u8; 3]> {
        let bytes = self.take(3)?;
        Ok([bytes[0], bytes[1], bytes[2]])
    }

    /// An XNA `Vector2`: two little-endian floats.
    pub fn vec2(&mut self) -> Result<(f32, f32)> {
        Ok((self.f32()?, self.f32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_primitives() {
        let mut r = PacketReader::new(&[0x34, 0x12, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.i32().unwrap(), 0x1234_5678);
        assert!(r.is_empty());
    }

    #[test]
    fn bool_is_any_non_zero_byte() {
        let mut r = PacketReader::new(&[0, 1, 0xFF]);
        assert!(!r.bool().unwrap());
        assert!(r.bool().unwrap());
        assert!(r.bool().unwrap());
    }

    #[test]
    fn var_u32_matches_dotnet_7bit_encoding() {
        // Boundaries where the encoding grows by a byte.
        for (bytes, expect) in [
            (vec![0x00], 0u32),
            (vec![0x7F], 127),
            (vec![0x80, 0x01], 128),
            (vec![0xFF, 0x7F], 16_383),
            (vec![0x80, 0x80, 0x01], 16_384),
            (vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F], u32::MAX),
        ] {
            let mut r = PacketReader::new(&bytes);
            assert_eq!(r.var_u32().unwrap(), expect, "decoding {bytes:02X?}");
            assert!(r.is_empty());
        }
    }

    #[test]
    fn var_u32_rejects_overlong_encodings() {
        // A sixth continuation byte, and a fifth byte carrying bits past 2^32.
        for bytes in [
            vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x01],
            vec![0xFF, 0xFF, 0xFF, 0xFF, 0x10],
        ] {
            let mut r = PacketReader::new(&bytes);
            assert!(
                matches!(r.var_u32(), Err(ProtoError::BadVarInt { .. })),
                "should reject {bytes:02X?}"
            );
        }
    }

    #[test]
    fn reads_strings_longer_than_127_bytes() {
        // The single-byte length case is the one everybody gets right; 128+ is where the
        // continuation bit starts to matter.
        let text = "w".repeat(300);
        let mut buf = vec![0xAC, 0x02]; // 300 as a 7-bit encoded int
        buf.extend_from_slice(text.as_bytes());

        let mut r = PacketReader::new(&buf);
        assert_eq!(r.string().unwrap(), text);
        assert!(r.is_empty());
    }

    #[test]
    fn string_length_counts_bytes_not_chars() {
        // "é" is two UTF-8 bytes each, so this three-character string declares five bytes.
        let text = "éée";
        assert_eq!(text.chars().count(), 3);
        assert_eq!(text.len(), 5);

        let mut buf = vec![text.len() as u8];
        buf.extend_from_slice(text.as_bytes());

        let mut r = PacketReader::new(&buf);
        assert_eq!(r.string().unwrap(), text);
        assert!(r.is_empty());
    }

    #[test]
    fn truncated_reads_report_offset_and_shortfall() {
        let mut r = PacketReader::new(&[0x01, 0x02]);
        assert_eq!(
            r.i32(),
            Err(ProtoError::Eof {
                offset: 0,
                needed: 4,
                available: 2
            })
        );
    }

    #[test]
    fn truncated_string_body_is_an_error_not_a_panic() {
        // Declares 200 bytes but supplies three.
        let mut r = PacketReader::new(&[0xC8, 0x01, b'a', b'b', b'c']);
        assert!(matches!(r.string(), Err(ProtoError::Eof { .. })));
    }
}
