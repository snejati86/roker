mod ring_buffer;

use crate::error::{Error, Result};
use crate::topic::TopicMatcher;
use crate::{BrokerConfig, BrokerStats, ClientId, Message};
use parking_lot::{Mutex, RwLock};
use ring_buffer::RingBuffer;
use shared_memory::{Shmem, ShmemConf};
use std::collections::HashMap;
use std::ffi::CString;
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
    read_position: usize, // Track read position per client
}

/// A thread-safe wrapper for shared memory
struct SharedMemory {
    shmem: Mutex<Shmem>,
    name: String,
}

unsafe impl Send for SharedMemory {}
unsafe impl Sync for SharedMemory {}

impl SharedMemory {
    /// Create new shared memory
    fn new(name: &str, size: usize) -> Result<Self> {
        // Convert name to CString for libc
        let c_name = CString::new(name.as_bytes()).map_err(|_| {
            Error::SharedMemoryCreation(shared_memory::ShmemError::MapCreateFailed(0))
        })?;

        // Try to unlink any existing shared memory
        unsafe {
            libc::shm_unlink(c_name.as_ptr());
        }

        // Create new shared memory with platform-specific configuration
        let shmem = if cfg!(target_os = "macos") {
            // On macOS, use named shared memory with a fixed path prefix
            let shm_path = format!("/tmp/{}", name);
            let c_path = CString::new(shm_path.as_bytes()).map_err(|_| {
                Error::SharedMemoryCreation(shared_memory::ShmemError::MapCreateFailed(0))
            })?;

            // Try to unlink any existing shared memory
            unsafe {
                libc::shm_unlink(c_path.as_ptr());
            }

            ShmemConf::new()
                .size(size)
                .os_id(&shm_path)
                .create()
                .map_err(Error::SharedMemoryCreation)?
        } else {
            ShmemConf::new()
                .size(size)
                .os_id(name)
                .create()
                .map_err(Error::SharedMemoryCreation)?
        };

        Ok(Self {
            shmem: Mutex::new(shmem),
            name: name.to_string(),
        })
    }

    /// Connect to existing shared memory
    fn connect(name: &str) -> Result<Self> {
        // Connect to shared memory with platform-specific configuration
        let shmem = if cfg!(target_os = "macos") {
            // On macOS, use named shared memory with a fixed path prefix
            let shm_path = format!("/tmp/{}", name);
            ShmemConf::new()
                .os_id(&shm_path)
                .open()
                .map_err(Error::SharedMemoryCreation)?
        } else {
            ShmemConf::new()
                .os_id(name)
                .open()
                .map_err(Error::SharedMemoryCreation)?
        };

        Ok(Self {
            shmem: Mutex::new(shmem),
            name: name.to_string(),
        })
    }

    /// Get a pointer to the shared memory
    unsafe fn as_ptr(&self) -> *mut u8 {
        self.shmem.lock().as_ptr()
    }

    /// Get the size of the shared memory
    fn len(&self) -> usize {
        self.shmem.lock().len()
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        // Clean up shared memory on drop
        if let Ok(c_name) = CString::new(self.name.as_bytes()) {
            unsafe {
                libc::shm_unlink(c_name.as_ptr());
            }
        }
    }
}

/// The broker manages the shared memory and client subscriptions
pub struct Broker {
    #[allow(dead_code)]
    shm: Arc<SharedMemory>,
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

        let shm = Arc::new(SharedMemory::new(&config.name, total_size)?);
        let ptr = unsafe { shm.as_ptr() };
        let ring_buffer =
            unsafe { Arc::new(RingBuffer::from_raw_parts(ptr, config.buffer_size, false)?) };

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

        let shm = Arc::new(SharedMemory::connect(name)?);
        let ptr = unsafe { shm.as_ptr() };
        let size = shm.len();
        let ring_buffer = unsafe { Arc::new(RingBuffer::from_raw_parts(ptr, size, false)?) };

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

        let mut subs = self.subscriptions.write();

        if subs.len() >= self.config.max_clients {
            error!("Client limit exceeded: {}", self.config.max_clients);
            return Err(Error::ClientLimitExceeded);
        }

        debug!("Registering new client: {} ({})", name, client_id.as_str());

        subs.insert(
            client_id.clone(),
            ClientInfo {
                patterns: Vec::new(),
                last_seen: std::time::Instant::now(),
                read_position: self.ring_buffer.oldest_read_pos(), // Start at current write position
            },
        );

        Ok(client_id)
    }

    /// Unregister a client
    pub fn unregister_client(&self, client_id: &ClientId) -> Result<()> {
        info!("Unregistering client: {}", client_id.as_str());
        let mut subs = self.subscriptions.write();
        if subs.remove(client_id).is_none() {
            warn!(
                "Attempted to unregister non-existent client: {}",
                client_id.as_str()
            );
            return Err(Error::ClientNotFound(client_id.as_str()));
        }
        Ok(())
    }

    /// Subscribe to a topic pattern
    pub fn subscribe(&self, client_id: &ClientId, pattern: &str) -> Result<()> {
        let mut subs = self.subscriptions.write();

        let client = subs
            .get_mut(client_id)
            .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;

        if client.patterns.len() >= self.config.max_subscriptions_per_client {
            error!(
                "Subscription limit exceeded for client {}: {}",
                client_id.as_str(),
                self.config.max_subscriptions_per_client
            );
            return Err(Error::SubscriptionLimitExceeded(client_id.as_str()));
        }

        debug!(
            "Client {} subscribing to pattern: {}",
            client_id.as_str(),
            pattern
        );
        client.patterns.push(TopicMatcher::new(pattern));
        client.last_seen = std::time::Instant::now();

        Ok(())
    }

    /// Unsubscribe from a topic pattern
    pub fn unsubscribe(&self, client_id: &ClientId, pattern: &str) -> Result<()> {
        let mut subs = self.subscriptions.write();

        let client = subs
            .get_mut(client_id)
            .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;

        debug!(
            "Client {} unsubscribing from pattern: {}",
            client_id.as_str(),
            pattern
        );
        let matcher = TopicMatcher::new(pattern);
        client.patterns.retain(|p| p != &matcher);
        client.last_seen = std::time::Instant::now();

        Ok(())
    }

    /// Publish a message
    pub fn publish(&self, message: Message) -> Result<()> {
        debug!("Publishing message to topic: {}", message.topic.name());

        let serialized =
            bincode::serialize(&message).map_err(|e| Error::SerializationError(e.to_string()))?;

        self.ring_buffer.write(&serialized)?;
        self.counters
            .messages_published
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Receive a message for a specific client
    pub fn receive(&self, client_id: &ClientId) -> Result<Message> {
        // Get client info and patterns
        let (patterns, read_pos) = {
            let subs = self.subscriptions.read();
            let client = subs
                .get(client_id)
                .ok_or_else(|| Error::ClientNotFound(client_id.as_str()))?;
            (client.patterns.clone(), client.read_position)
        };

        // Read message from client's position
        let (data, new_pos) = self.ring_buffer.read_message(read_pos)?;

        let message: Message =
            bincode::deserialize(&data).map_err(|e| Error::DeserializationError(e.to_string()))?;

        // Check if message matches any patterns
        if patterns.iter().any(|p| p.matches(message.topic.name())) {
            // Update client's read position
            if let Some(client) = self.subscriptions.write().get_mut(client_id) {
                client.read_position = new_pos;
                client.last_seen = std::time::Instant::now();
            }

            debug!(
                "Delivered message on topic {} to client {}",
                message.topic.name(),
                client_id.as_str()
            );

            self.counters
                .messages_delivered
                .fetch_add(1, Ordering::Relaxed);
            Ok(message)
        } else {
            // If message doesn't match, advance read position and try again
            if let Some(client) = self.subscriptions.write().get_mut(client_id) {
                client.read_position = new_pos;
            }
            // Return BufferEmpty to indicate no matching message was found
            // This will cause the caller to retry
            Err(Error::BufferEmpty)
        }
    }

    /// Get broker statistics
    pub fn stats(&self) -> BrokerStats {
        let subs = self.subscriptions.read();
        let total_subscriptions = subs.values().map(|client| client.patterns.len()).sum();

        BrokerStats {
            connected_clients: subs.len(),
            total_subscriptions,
            messages_published: self.counters.messages_published.load(Ordering::Relaxed),
            messages_delivered: self.counters.messages_delivered.load(Ordering::Relaxed),
            buffer_usage: self.ring_buffer.usage(),
        }
    }

    /// Get the current write position (for debugging)
    pub fn debug_write_pos(&self) -> usize {
        self.ring_buffer.write_pos()
    }

    /// Get the current read position (for debugging)
    pub fn debug_read_pos(&self) -> usize {
        self.ring_buffer.oldest_read_pos()
    }

    /// Get the buffer size (for debugging)
    pub fn debug_buffer_size(&self) -> usize {
        self.config.buffer_size
    }
}
