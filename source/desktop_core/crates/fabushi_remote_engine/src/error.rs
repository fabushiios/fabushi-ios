use thiserror::Error;

pub type EngineResult<T> = std::result::Result<T, RemoteEngineError>;

#[derive(Debug, Error)]
pub enum RemoteEngineError {
    #[error("remote engine feature unavailable on this host: {0}")]
    Unavailable(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("remote control preempted by local user or kill switch: {0}")]
    Preempted(String),

    #[error("remote session rejected: {0}")]
    SessionRejected(String),

    #[error("nested fabushi mux session blocked (FABUSHI_IN_MUX=1): {0}")]
    NestedMuxBlocked(String),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("operation timed out: {0}")]
    Timeout(String),

    #[error("local IPC error: {0}")]
    Ipc(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
