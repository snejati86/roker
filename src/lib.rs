//! A high-performance shared memory broker for pub/sub communication.
//!
//! This library provides a broker that uses shared memory for fast inter-process
//! communication using a publish/subscribe pattern.

pub mod broker;
pub mod client;
pub mod error;
mod topic;

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

pub use broker::Broker;
pub use client::{Publisher, Subscriber};
pub use error::{Error, Result};

/// Configuration for creating a new broker
#[derive(Debug, Clone)]
pub struct BrokerConfig {
    /// Name of the shared memory segment
    pub name: String,
    /// Size of the buffer in bytes
    pub buffer_size: usize,
    /// Maximum number of clients that can connect
    pub max_clients: usize,
    /// Maximum subscriptions per client
    pub max_subscriptions_per_client: usize,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            name: "roker".to_string(),
            buffer_size: 64 * 1024 * 1024, // 64MB
            max_clients: 1000,
            max_subscriptions_per_client: 100,
        }
    }
}

/// Unique identifier for a client
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientId(Uuid);

impl Default for ClientId {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientId {
    /// Create a new random client ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the string representation of the client ID
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

/// Topic for message routing
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct Topic {
    name: String,
}

impl Topic {
    /// Create a new topic
    pub fn new(name: &str) -> Result<Self> {
        if name.is_empty() {
            return Err(Error::InvalidTopic("Topic name cannot be empty".into()));
        }
        if name.len() > MAX_TOPIC_LENGTH {
            return Err(Error::TopicTooLong);
        }
        Ok(Self {
            name: name.to_string(),
        })
    }

    /// Get the topic name
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A message in the pub/sub system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The topic this message belongs to
    pub topic: Topic,
    /// The message payload
    pub payload: Vec<u8>,
    /// Timestamp when the message was created
    pub timestamp: SystemTime,
}

impl Message {
    /// Create a new message
    pub fn new(topic: Topic, payload: Vec<u8>) -> Self {
        Self {
            topic,
            payload,
            timestamp: SystemTime::now(),
        }
    }
}

/// Statistics about the broker's operation
#[derive(Debug, Clone)]
pub struct BrokerStats {
    /// Number of currently connected clients
    pub connected_clients: usize,
    /// Total number of active subscriptions
    pub total_subscriptions: usize,
    /// Total number of messages published
    pub messages_published: u64,
    /// Total number of messages delivered
    pub messages_delivered: u64,
    /// Current buffer usage (0.0 - 1.0)
    pub buffer_usage: f32,
}

/// Maximum length for topic names
pub const MAX_TOPIC_LENGTH: usize = 256;
