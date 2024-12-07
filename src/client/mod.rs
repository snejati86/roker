use crate::error::Result;
use crate::{Broker, ClientId, Message, Topic};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

/// A client that can publish messages to topics
#[derive(Clone)]
pub struct Publisher {
    broker: Arc<Broker>,
    client_id: ClientId,
}

impl Publisher {
    /// Create a new publisher
    pub fn new(broker: Arc<Broker>, client_id: ClientId) -> Self {
        debug!("Creating new publisher with client ID: {}", client_id.as_str());
        Self { broker, client_id }
    }

    /// Publish a message to a topic
    pub fn publish(&self, topic: &Topic, payload: &[u8]) -> Result<()> {
        let message = Message::new(topic.clone(), payload.to_vec());
        debug!(
            "Publishing message to topic {} (size: {} bytes)",
            topic.name(),
            payload.len()
        );
        self.broker.publish(message)
    }

    /// Publish multiple messages in batch
    pub fn publish_batch(&self, messages: &[(&Topic, &[u8])]) -> Result<()> {
        debug!("Publishing batch of {} messages", messages.len());
        for (topic, payload) in messages {
            let message = Message::new((*topic).clone(), payload.to_vec());
            self.broker.publish(message)?;
        }
        Ok(())
    }
}

/// A client that can subscribe to topics and receive messages
#[derive(Clone)]
pub struct Subscriber {
    broker: Arc<Broker>,
    client_id: ClientId,
}

impl Subscriber {
    /// Create a new subscriber
    pub fn new(broker: Arc<Broker>, client_id: ClientId) -> Self {
        debug!("Creating new subscriber with client ID: {}", client_id.as_str());
        Self { broker, client_id }
    }

    /// Subscribe to a topic pattern
    pub fn subscribe(&self, pattern: &str) -> Result<()> {
        info!("Subscribing to pattern: {}", pattern);
        self.broker.subscribe(&self.client_id, pattern)
    }

    /// Unsubscribe from a topic pattern
    pub fn unsubscribe(&self, pattern: &str) -> Result<()> {
        info!("Unsubscribing from pattern: {}", pattern);
        self.broker.unsubscribe(&self.client_id, pattern)
    }

    /// Receive a single message
    pub fn receive(&self) -> Result<Message> {
        self.broker.receive(&self.client_id)
    }

    /// Receive multiple messages
    pub fn receive_batch(&self, max_messages: usize) -> Result<Vec<Message>> {
        debug!("Attempting to receive batch of up to {} messages", max_messages);
        let mut messages = Vec::with_capacity(max_messages);
        
        for _ in 0..max_messages {
            match self.receive() {
                Ok(message) => messages.push(message),
                Err(crate::error::Error::BufferEmpty) => break,
                Err(e) => {
                    error!("Error receiving message: {}", e);
                    return Err(e);
                }
            }
        }

        Ok(messages)
    }

    /// Receive a message with timeout
    pub fn receive_timeout(&self, timeout: Duration) -> Result<Message> {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            match self.receive() {
                Ok(message) => return Ok(message),
                Err(crate::error::Error::BufferEmpty) => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(crate::error::Error::Timeout)
    }
} 