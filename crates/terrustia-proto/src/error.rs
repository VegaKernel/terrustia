use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProtoError>;

/// Every way a Terraria packet can fail to parse or serialise.
///
/// Errors carry the byte offset they occurred at: a client that stalls silently is the normal
/// failure mode for this protocol, so the offset is usually the only clue about which field of a
/// long packet drifted.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtoError {
    #[error(
        "unexpected end of packet at offset {offset}: needed {needed} byte(s), {available} available"
    )]
    Eof {
        offset: usize,
        needed: usize,
        available: usize,
    },

    #[error("malformed 7-bit encoded length at offset {offset}")]
    BadVarInt { offset: usize },

    #[error("invalid UTF-8 in string at offset {offset}")]
    Utf8 { offset: usize },

    #[error("packet is {len} bytes, exceeding the {max}-byte frame limit")]
    FrameTooLarge { len: usize, max: usize },

    #[error("frame is {len} bytes, too short to hold a length prefix and message id")]
    FrameTooShort { len: usize },

    #[error("unknown message id {id}")]
    UnknownMessageId { id: u8 },

    #[error("{field} has out-of-range value {value}")]
    OutOfRange { field: &'static str, value: i64 },

    #[error("deflate stream could not be decoded: {0}")]
    Deflate(String),
}
