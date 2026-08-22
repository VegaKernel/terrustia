use bytes::{Buf, Bytes, BytesMut};

use crate::error::ClientError;

/// Smallest legal frame: a length prefix and a message id, with no payload.
pub const MIN_FRAME_LEN: usize = 3;

/// One decoded packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: u8,
    pub payload: Bytes,
}

/// Pull one frame out of a buffer, if a whole one is present.
///
/// The client keeps its own copy of the framing rather than depending on the server crate, so that
/// a bug in one cannot mask the same bug in the other when they are tested against each other.
pub fn decode(buf: &mut BytesMut) -> Result<Option<Frame>, ClientError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
    if len < MIN_FRAME_LEN {
        return Err(ClientError::Desynchronised { len });
    }
    if buf.len() < len {
        buf.reserve(len - buf.len());
        return Ok(None);
    }

    let mut frame = buf.split_to(len);
    frame.advance(2);
    let id = frame[0];
    frame.advance(1);
    Ok(Some(Frame {
        id,
        payload: frame.freeze(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_only_when_the_whole_frame_is_present() {
        let whole = [6u8, 0, 13, 1, 2, 3];
        let mut buf = BytesMut::new();
        for (i, byte) in whole.iter().enumerate() {
            buf.extend_from_slice(&[*byte]);
            let got = decode(&mut buf).unwrap();
            if i + 1 < whole.len() {
                assert!(got.is_none());
            } else {
                let frame = got.unwrap();
                assert_eq!(frame.id, 13);
                assert_eq!(&frame.payload[..], &[1, 2, 3]);
            }
        }
    }

    #[test]
    fn rejects_an_impossible_length() {
        let mut buf = BytesMut::from(&[2u8, 0, 0, 0][..]);
        assert!(decode(&mut buf).is_err());
    }
}
