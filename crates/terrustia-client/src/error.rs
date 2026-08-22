use thiserror::Error;

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol: {0}")]
    Proto(#[from] terrustia_proto::ProtoError),

    #[error("the server closed the connection")]
    Closed,

    #[error("frame declares {len} bytes, which cannot hold its own header")]
    Desynchronised { len: usize },

    #[error("the server rejected the connection: {reason}")]
    Kicked { reason: String },

    #[error("timed out after {seconds}s waiting for {expected}")]
    Timeout { expected: String, seconds: u64 },

    #[error("the server sent {got} when {expected} was expected")]
    Unexpected { expected: String, got: String },
}
