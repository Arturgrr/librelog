use serde::{Serialize, ser::Serializer};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store2(#[from] tauri_plugin_store2::Error),
    #[error(transparent)]
    Updater(#[from] tauri_plugin_updater::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Config(#[from] serde_json::Error),
    #[error("update download failed with status {0}")]
    DownloadStatus(u16),
    #[error("invalid update download response")]
    InvalidDownloadResponse,
    #[error("update signature verification failed")]
    InvalidSignature,
    #[error("cache path unavailable")]
    CachePathUnavailable,
    #[error("cached update not found")]
    CachedUpdateNotFound,
    #[error("update not available")]
    UpdateNotAvailable,
    #[error("version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: String, actual: String },
    #[error("cached update {version} is not newer than current {current}")]
    UpdateNotNewer { version: String, current: String },
    #[error("this installation updates through its package, not in place")]
    SelfUpdateUnsupported,
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}
