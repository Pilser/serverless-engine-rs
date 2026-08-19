use thiserror::Error;

#[derive(Error, Debug)]
pub enum WrapperError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("ZeroClaw RPC error: {0}")]
    ZeroClaw(String),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for WrapperError {
    fn from(e: anyhow::Error) -> Self {
        WrapperError::Other(e.to_string())
    }
}
