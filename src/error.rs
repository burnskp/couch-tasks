use thiserror::Error;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    User(String),

    #[error("")]
    UserNoMessage,

    #[error("Storage error: {0}")]
    Storage(#[from] rusqlite::Error),

    #[error("Storage error: {0}")]
    StorageIo(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    StorageMessage(String),

    #[error("Editor error: {0}")]
    Editor(String),

    #[error("Sync error: {0}")]
    Sync(String),

    #[error("{0}")]
    SyncBadRequest(String),

    #[error("{0}")]
    SyncConflict(String),
}

impl AppError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::User(_) | Self::UserNoMessage => 1,
            Self::Storage(_) | Self::StorageIo(_) | Self::StorageMessage(_) => 2,
            Self::Editor(_) => 3,
            Self::Sync(_) | Self::SyncBadRequest(_) | Self::SyncConflict(_) => 4,
        }
    }
}
