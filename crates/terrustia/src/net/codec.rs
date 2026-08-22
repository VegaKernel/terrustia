use bytes::{Buf, Bytes, BytesMut};
use terrustia_proto::MAX_FRAME_LEN;
use thiserror::Error;
use tokio_util::codec::{Decoder, Encoder};

/// Smallest legal frame: the 2-byte length prefix plus a message id, with no payload.
pub const MIN_FRAME_LEN: usize = 3;

/// One decoded packet: its message id and the payload that follows it.
///
/// The length prefix is stripped here; nothing downstream needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: u8,
    pub payload: Bytes,
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("frame declares {len} bytes, below the {MIN_FRAME_LEN}-byte minimum")]
    FrameTooShort { len: usize },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Terraria's framing: `[u16 length][u8 message id][payload]`, little-endian, where the length
/// counts the entire frame *including its own two bytes*.
#[derive(Debug, Default, Clone, Copy)]
pub struct TerrariaCodec;

impl Decoder for TerrariaCodec {
    type Item = Frame;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, CodecError> {
        if src.len() < 2 {
            // Not even a length yet. Ask for the rest of a prefix so the read buffer is sized
            // sensibly rather than growing a byte at a time.
            src.reserve(MIN_FRAME_LEN - src.len());
            return Ok(None);
        }

        let len = u16::from_le_bytes([src[0], src[1]]) as usize;
        if len < MIN_FRAME_LEN {
            // A length that cannot cover its own header means the stream is desynchronised; there
            // is no safe number of bytes to skip, so the connection has to go.
            return Err(CodecError::FrameTooShort { len });
        }

        if src.len() < len {
            src.reserve(len - src.len());
            return Ok(None);
        }

        let mut frame = src.split_to(len);
        frame.advance(2); // length prefix
        let id = frame[0];
        frame.advance(1);

        Ok(Some(Frame {
            id,
            payload: frame.freeze(),
        }))
    }
}

/// Frames are built complete by `terrustia_proto::PacketWriter`, so encoding is a copy. Keeping it
/// that way lets a broadcast serialise once and hand the same `Bytes` to every recipient.
impl Encoder<Bytes> for TerrariaCodec {
    type Error = CodecError;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), CodecError> {
        debug_assert!(
            item.len() >= MIN_FRAME_LEN
                && u16::from_le_bytes([item[0], item[1]]) as usize == item.len(),
            "attempted to send a frame whose length prefix does not match its size",
        );
        debug_assert!(item.len() <= MAX_FRAME_LEN);

        dst.reserve(item.len());
        dst.extend_from_slice(&item);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::PacketWriter;

    fn frame_bytes(id: u8, payload: &[u8]) -> Bytes {
        let mut w = PacketWriter::new(id);
        w.bytes(payload);
        Bytes::from(w.finish().unwrap())
    }

    #[test]
    fn decodes_a_whole_frame() {
        let mut buf = BytesMut::from(&frame_bytes(7, &[1, 2, 3])[..]);
        let frame = TerrariaCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.id, 7);
        assert_eq!(&frame.payload[..], &[1, 2, 3]);
        assert!(buf.is_empty());
    }

    #[test]
    fn decodes_an_empty_payload_frame() {
        let mut buf = BytesMut::from(&frame_bytes(6, &[])[..]);
        let frame = TerrariaCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.id, 6);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn waits_for_the_rest_of_a_split_frame() {
        // TCP may deliver a frame one byte at a time; the decoder must yield nothing until the
        // final byte lands, then exactly one frame.
        let whole = frame_bytes(13, &[9; 20]);
        let mut codec = TerrariaCodec;
        let mut buf = BytesMut::new();

        for (i, byte) in whole.iter().enumerate() {
            buf.extend_from_slice(&[*byte]);
            let decoded = codec.decode(&mut buf).unwrap();
            if i + 1 < whole.len() {
                assert!(decoded.is_none(), "yielded a frame after {} bytes", i + 1);
            } else {
                let frame = decoded.expect("frame should complete on the last byte");
                assert_eq!(frame.id, 13);
                assert_eq!(&frame.payload[..], &[9; 20]);
            }
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn decodes_several_frames_coalesced_into_one_read() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&frame_bytes(1, b"Terraria279"));
        buf.extend_from_slice(&frame_bytes(3, &[0]));
        buf.extend_from_slice(&frame_bytes(6, &[]));

        let mut codec = TerrariaCodec;
        let ids: Vec<u8> = std::iter::from_fn(|| codec.decode(&mut buf).unwrap())
            .map(|f| f.id)
            .collect();

        assert_eq!(ids, vec![1, 3, 6]);
        assert!(buf.is_empty());
    }

    #[test]
    fn keeps_the_tail_of_a_partial_second_frame() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&frame_bytes(1, b"hi"));
        buf.extend_from_slice(&frame_bytes(4, &[1, 2, 3, 4])[..2]); // only the length prefix

        let mut codec = TerrariaCodec;
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap().id, 1);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(buf.len(), 2, "partial frame must stay buffered");
    }

    #[test]
    fn rejects_a_length_that_cannot_hold_its_own_header() {
        for len in [0u16, 1, 2] {
            let mut buf = BytesMut::new();
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&[0; 8]);
            assert!(
                matches!(
                    TerrariaCodec.decode(&mut buf),
                    Err(CodecError::FrameTooShort { .. })
                ),
                "length {len} should be rejected"
            );
        }
    }

    #[test]
    fn encodes_a_frame_verbatim() {
        let frame = frame_bytes(82, &[1, 0, 0]);
        let mut dst = BytesMut::new();
        TerrariaCodec.encode(frame.clone(), &mut dst).unwrap();
        assert_eq!(&dst[..], &frame[..]);
    }

    #[test]
    fn encode_then_decode_is_identity() {
        let mut codec = TerrariaCodec;
        let mut dst = BytesMut::new();
        codec.encode(frame_bytes(49, &[]), &mut dst).unwrap();
        codec.encode(frame_bytes(10, &[7; 300]), &mut dst).unwrap();

        assert_eq!(codec.decode(&mut dst).unwrap().unwrap().id, 49);
        let second = codec.decode(&mut dst).unwrap().unwrap();
        assert_eq!(second.id, 10);
        assert_eq!(second.payload.len(), 300);
        assert!(dst.is_empty());
    }
}
