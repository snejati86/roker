mod ring_buffer;

use crate::error::{Error, Result};
use crate::topic::TopicMatcher;
use crate::{BrokerConfig, BrokerStats, ClientId, Message, Topic};
use parking_lot::RwLock;
use ring_buffer::RingBuffer;
use shared_memory::{Shmem, ShmemConf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

const METADATA_SIZE: usize = 1024;

/// Statistics counters for the broker
struct Counters {
    messages_published: AtomicU64,
    messages_delivered: AtomicU64,
}

/// Client subscription information
struct ClientInfo {
    patterns: Vec<TopicMatcher>,
    last_seen: std::time::Instant,
}

/// The broker manages the shared memory and client subscriptions
pub struct Broker {
    shm: Arc<Shmem>,
    ring_buffer: Arc<RingBuffer>,
    subscriptions: Arc<RwLock<HashMap<ClientId, ClientInfo>>>,
    counters: Arc<Counters>,
    config: BrokerConfig,
}

impl Broker {
    /// Create a new broker instance
    pub fn new(config: BrokerConfig) -> Result<Self> {
        info!("Creating new broker with config: {:?}", config);
        
        if config.buffer_size == 0 {
            return Err(Error::InvalidConfig("Buffer size cannot be zero".into()));
        }

        let total_size = config.buffer_size + METADATA_SIZE;
        
        let shm = Arc::new(
            ShmemConf::new()
                .size(total_size)
                .os_id(&config.name)
                .create()
                .map_err(Error::SharedMemoryCreation)?,
        );

        debug!("Created shared memory segment of size {}", total_size);

        let ptr = shm.as_ptr();
        let ring_buffer = unsafe {
            Arc::new(RingBuffer::from_raw_parts(
                ptr as *mut u8,
                config.buffer_size,
            )?)
        };

        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let counters = Arc::new(Counters {
            messages_published: AtomicU64::new(0),
            messages_delivered: AtomicU64::new(0),
        });

        Ok(Self {
            shm,
            ring_buffer,
            subscriptions,
            counters,
            config,
        })
    }

    /// Connect to an existing broker
    pub fn connect(name: &str) -> Result<Self> {
        info!("Connecting to existing broker: {}", name);
        
        let shm = Arc::new(
            ShmemConf::new()
                .os_id(name)
                .open()
                .map_err(Error::SharedMemoryCreation)?,
        );

        let size = shm.len() - METADATA_SIZE;
        
        let ptr = shm.as_ptr();
        let ring_buffer = unsafe {
            Arc::new(RingBuffer::from_raw_parts(
                ptr as *mut u8,
                size,
            )?)
        };

        let config = BrokerConfig {
            name: name.to_string(),
            buffer_size: size,
            ..Default::default()
        };

        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let counters = Arc::new(Counters {
            messages_published: AtomicU64::new(0),
            messages_delivered: AtomicU64::new(0),
        });

        Ok(Self {
            shm,
            ring_buffer,
            subscriptions,
            counters,
            config,
        })
    }

    /// Register a new client
    pub fn register_client(&self, name: &str) -> Result<ClientId> {
        let client_id = ClientId::new();
        let subs = self.subscriptions.read();
        
        if subs.len() >= self.config.max_clients {
            error!("Client limit exceeded: {}", self.config.max_clients);
            return Err(Error::ClientLimitExceeded);
        }

        debug!("Registering new client: {} ({})", name, client_id.as_str());
        
        let mut subs = self.subscriptions.write();
        subs.insert(client_id.clone(), ClientInfo {
            patterns: Vec::new(),
            last_seen: std::time::Instant::now(),
        });

        Ok(client_id)
    }

    /// Unregister a client
    pub fn unregister_client(&self, client_id: &ClientId) -> Result<()> {
        info!("Unregistering client: {}", client_id.as_str());
        let mut subs = self.subscriptions.write();
        if subs.remove(client_id).is_none() {
            warn!("Attempted to unregister non-existent client: {}", client_id.as_str());
            return Err(Error::ClientNotFound(client_id.as_str()));
        }
        Ok(())
    }

    /// Subscribe to a topic pattern
    pub fn subscribe(&self, client_id: &ClientId, pattern: &str) -> Result<()> {
        let mut subs = self.subscriptions.write();
        
        let client = subs.get_mut(client_id)
            .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;

        if client.patterns.len() >= self.config.max_subscriptions_per_client {
            error!(
                "Subscription limit exceeded for client {}: {}",
                client_id.as_str(),
                self.config.max_subscriptions_per_client
            );
            return Err(Error::SubscriptionLimitExceeded(client_id.as_str()));
        }

        debug!("Client {} subscribing to pattern: {}", client_id.as_str(), pattern);
        client.patterns.push(TopicMatcher::new(pattern));
        client.last_seen = std::time::Instant::now();

        Ok(())
    }

    /// Unsubscribe from a topic pattern
    pub fn unsubscribe(&self, client_id: &ClientId, pattern: &str) -> Result<()> {
        let mut subs = self.subscriptions.write();
        
        let client = subs.get_mut(client_id)
            .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;

        debug!("Client {} unsubscribing from pattern: {}", client_id.as_str(), pattern);
        let matcher = TopicMatcher::new(pattern);
        client.patterns.retain(|p| p != &matcher);
        client.last_seen = std::time::Instant::now();

        Ok(())
    }

    /// Publish a message
    pub fn publish(&self, message: Message) -> Result<()> {
        debug!("Publishing message to topic: {}", message.topic.name());
        
        let serialized = bincode::serialize(&message)
            .map_err(|e| Error::SerializationError(e.to_string()))?;

        self.ring_buffer.write(&serialized)?;
        self.counters.messages_published.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Receive a message for a specific client
    pub fn receive(&self, client_id: &ClientId) -> Result<Message> {
        let mut subs = self.subscriptions.write();
        let client = subs.get_mut(client_id)
            .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;

        let mut temp_buf = vec![0u8; self.ring_buffer.size()];
        self.ring_buffer.read(&mut temp_buf)?;

        let message: Message = bincode::deserialize(&temp_buf)
            .map_err(|e| Error::DeserializationError(e.to_string()))?;

        if client.patterns.iter().any(|p| p.matches(message.topic.name())) {
            debug!(
                "Delivering message on topic {} to client {}",
                message.topic.name(),
                client_id.as_str()
            );
            client.last_seen = std::time::Instant::now();
            self.counters.messages_delivered.fetch_add(1, Ordering::Relaxed);
            Ok(message)
        } else {
            Err(Error::BufferEmpty)
        }
    }

    /// Get broker statistics
    pub fn stats(&self) -> BrokerStats {
        let subs = self.subscriptions.read();
        let total_subscriptions = subs.values()
            .map(|client| client.patterns.len())
            .sum();

        BrokerStats {
            connected_clients: subs.len(),
            total_subscriptions,
            messages_published: self.counters.messages_published.load(Ordering::Relaxed),
            messages_delivered: self.counters.messages_delivered.load(Ordering::Relaxed),
            buffer_usage: self.ring_buffer.usage(),
        }
    }
} 