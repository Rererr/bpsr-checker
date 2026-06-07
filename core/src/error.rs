use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Capture error: {0}")]
    Capture(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Engine error: {0}")]
    Engine(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),
    #[error("Lock poisoned: {0}")]
    LockPoisoned(String),
}

pub type AppResult<T> = Result<T, AppError>;
