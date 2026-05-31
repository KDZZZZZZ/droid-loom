use thiserror::Error;

pub type AgentCoreResult<T> = Result<T, AgentCoreError>;

#[derive(Debug, Error)]
pub enum AgentCoreError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("recoverable error: {0}")]
    Recoverable(String),

    #[error("fatal error: {0}")]
    Fatal(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
