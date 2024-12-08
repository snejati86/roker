use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to create shared memory segment: {0}")]
    SharedMemoryCreation(#[from] shared_memory::ShmemError),

    #[error("Buffer is full")]
    BufferFull,

    #[error("Buffer is empty")]
    BufferEmpty,

    #[error("Invalid buffer size")]
    InvalidBufferSize,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Topic name too long")]
    TopicTooLong,

    #[error("Invalid topic name: {0}")]
    InvalidTopic(String),

    #[error("Failed to acquire broker lock")]
    LockError,

    #[error("Failed to serialize message: {0}")]
    SerializationError(String),

    #[error("Failed to deserialize message: {0}")]
    DeserializationError(String),

    #[error("Client limit exceeded")]
    ClientLimitExceeded,

    #[error("Client not found: {0}")]
    ClientNotFound(String),

    #[error("Subscription limit exceeded for client: {0}")]
    SubscriptionLimitExceeded(String),

    #[error("Operation timeout")]
    Timeout,

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Buffer too small")]
    BufferTooSmall,
}

pub type Result<T> = std::result::Result<T, Error>;
