use crate::error::{Error, Result};
use parking_lot::Mutex;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_WAIT_TIME: Duration = Duration::from_secs(5);

/// A thread-safe wrapper for raw memory pointer
struct SharedPtr {
    ptr: *mut u8,
}

// Implement Send and Sync for SharedPtr
unsafe impl Send for SharedPtr {}
unsafe impl Sync for SharedPtr {}

impl SharedPtr {
    /// Create a new SharedPtr from a raw pointer
    unsafe fn new(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Get a mutable reference to the underlying memory
    pub fn get_mut(&self) -> *mut u8 {
        self.ptr
    }

    pub fn get(&self) -> *const u8 {
        self.ptr
    }
}

/// Represents a subscriber with its own read position and last sequence number
pub struct Subscriber {
    pub read_pos: usize,
    last_seq: u64,
}

impl Subscriber {
    fn new(initial_pos: usize) -> Self {
        Self {
            read_pos: initial_pos,
            last_seq: 0,
        }
    }

    /// Read data from the ring buffer
    pub fn read(&mut self, ring_buffer: &RingBuffer) -> Result<Vec<u8>> {
        ring_buffer.read(&mut self.read_pos, &mut self.last_seq)
    }
}

/// A fixed-size ring buffer implementation using shared memory
pub struct RingBuffer {
    data: SharedPtr,
    pub size: usize,
    write_pos: AtomicUsize,
    sequence: AtomicU64,
    lock: Mutex<()>,
    subscribers: Mutex<Vec<Arc<Mutex<Subscriber>>>>,
    owns_data: bool,
}

unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    #[cfg(test)]
    /// Create a new ring buffer with the given size
    pub fn new(size: usize) -> Result<Self> {
        if size == 0 || !size.is_power_of_two() {
            return Err(Error::InvalidBufferSize);
        }

        let data = vec![0u8; size];
        let ptr = Box::into_raw(data.into_boxed_slice()) as *mut u8;

        Ok(Self {
            data: unsafe { SharedPtr::new(ptr) },
            size,
            write_pos: AtomicUsize::new(0),
            sequence: AtomicU64::new(0),
            lock: Mutex::new(()),
            subscribers: Mutex::new(Vec::new()),
            owns_data: true,
        })
    }

    /// Create a ring buffer from raw parts
    ///
    /// # Arguments
    /// * `ptr` - Pointer to the buffer memory
    /// * `size` - Size of the buffer (must be power of 2)
    /// * `owns_memory` - Whether this RingBuffer should take ownership of the memory
    ///
    /// # Safety
    /// - The caller must ensure the pointer is valid and properly aligned
    /// - If `owns_memory` is true, the memory must have been allocated with the same
    ///   allocator that Rust's global allocator would use
    /// - The caller must ensure the memory won't be freed while this RingBuffer exists
    pub unsafe fn from_raw_parts(ptr: *mut u8, size: usize, owns_memory: bool) -> Result<Self> {
        if size == 0 || !size.is_power_of_two() {
            return Err(Error::InvalidBufferSize);
        }

        Ok(Self {
            data: SharedPtr::new(ptr),
            size,
            write_pos: AtomicUsize::new(0),
            sequence: AtomicU64::new(0),
            lock: Mutex::new(()),
            subscribers: Mutex::new(Vec::new()),
            owns_data: owns_memory,
        })
    }

    /// Register a new subscriber and return it
    pub fn add_subscriber(&self) -> Arc<Mutex<Subscriber>> {
        let subscriber = Arc::new(Mutex::new(Subscriber::new(
            self.write_pos.load(Ordering::Acquire),
        )));
        let mut subscribers = self.subscribers.lock();
        subscribers.push(Arc::clone(&subscriber));
        subscriber
    }

    /// Write data to the ring buffer
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let _total_size = size_of::<u64>() + size_of::<u32>() + data.len();
        let mut _guard = self.lock.lock();

        let write_pos = self.write_pos.load(Ordering::Relaxed);

        // The buffer might overwrite old data if not enough space is available
        let mut current_pos = write_pos;

        unsafe {
            // Write sequence number
            let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
            let seq_bytes = seq.to_le_bytes();
            self.write_bytes(&seq_bytes, &mut current_pos)?;

            // Write length prefix
            let length_bytes = (data.len() as u32).to_le_bytes();
            self.write_bytes(&length_bytes, &mut current_pos)?;

            // Write data
            self.write_bytes(data, &mut current_pos)?;

            // Update write position
            std::sync::atomic::fence(Ordering::Release);
            self.write_pos.store(current_pos, Ordering::Release);
        }

        Ok(())
    }

    /// Helper to write bytes handling wrap-around
    unsafe fn write_bytes(&self, src: &[u8], current_pos: &mut usize) -> Result<()> {
        let end_pos = *current_pos + src.len();
        if end_pos > self.size {
            let first_chunk = self.size - *current_pos;
            let ptr = self.data.get_mut().add(*current_pos);
            std::ptr::copy_nonoverlapping(src.as_ptr(), ptr, first_chunk);

            let remaining = src.len() - first_chunk;
            let ptr = self.data.get_mut();
            std::ptr::copy_nonoverlapping(src.as_ptr().add(first_chunk), ptr, remaining);

            *current_pos = remaining;
        } else {
            let ptr = self.data.get_mut().add(*current_pos);
            std::ptr::copy_nonoverlapping(src.as_ptr(), ptr, src.len());
            *current_pos = (*current_pos + src.len()) % self.size;
        }
        Ok(())
    }

    /// Read data from the ring buffer starting from a given position and sequence number
    pub fn read(&self, read_pos: &mut usize, last_seq: &mut u64) -> Result<Vec<u8>> {
        let start = Instant::now();
        let mut _guard = self.lock.lock();

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Acquire);

            if *read_pos == write_pos {
                // Buffer is empty
                return Err(Error::BufferEmpty);
            }

            // Read sequence number
            let mut seq_bytes = [0u8; 8];
            self.read_bytes(&mut seq_bytes, *read_pos)?;
            let seq = u64::from_le_bytes(seq_bytes);

            // Check if data has been overwritten
            if seq <= *last_seq {
                // Data has been overwritten, need to adjust read_pos
                *read_pos = write_pos;
                *last_seq = seq;
                return Err(Error::DataOverwritten);
            }

            *last_seq = seq;
            *read_pos = (*read_pos + size_of::<u64>()) % self.size;

            // Read length prefix
            let mut length_bytes = [0u8; 4];
            self.read_bytes(&mut length_bytes, *read_pos)?;
            let data_len = u32::from_le_bytes(length_bytes) as usize;
            *read_pos = (*read_pos + size_of::<u32>()) % self.size;

            // Read data
            let mut data = vec![0u8; data_len];
            self.read_bytes(&mut data, *read_pos)?;
            *read_pos = (*read_pos + data_len) % self.size;

            return Ok(data);
        }

        Err(Error::Timeout)
    }

    /// Helper method to read bytes handling wrap-around
    fn read_bytes(&self, buf: &mut [u8], read_pos: usize) -> Result<()> {
        let end_pos = read_pos + buf.len();

        unsafe {
            if end_pos > self.size {
                let first_chunk = self.size - read_pos;
                let ptr = self.data.get().add(read_pos);
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), first_chunk);

                let remaining = buf.len() - first_chunk;
                let ptr = self.data.get();
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr().add(first_chunk), remaining);
            } else {
                let ptr = self.data.get().add(read_pos);
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), buf.len());
            }
        }

        Ok(())
    }

    /// Get the current write position
    pub fn write_pos(&self) -> usize {
        self.write_pos.load(Ordering::Acquire)
    }

    /// Read a message from the ring buffer at the given position
    pub fn read_message(&self, read_pos: usize) -> Result<(Vec<u8>, usize)> {
        let mut current_pos = read_pos;

        // Read sequence number
        let mut seq_bytes = [0u8; 8];
        self.read_bytes(&mut seq_bytes, current_pos)?;
        current_pos = (current_pos + size_of::<u64>()) % self.size;

        // Read length prefix
        let mut length_bytes = [0u8; 4];
        self.read_bytes(&mut length_bytes, current_pos)?;
        let data_len = u32::from_le_bytes(length_bytes) as usize;
        current_pos = (current_pos + size_of::<u32>()) % self.size;

        // Read data
        let mut data = vec![0u8; data_len];
        self.read_bytes(&mut data, current_pos)?;
        current_pos = (current_pos + data_len) % self.size;

        Ok((data, current_pos))
    }

    /// Get the buffer usage percentage
    pub fn usage(&self) -> f32 {
        let write_pos = self.write_pos();
        let oldest_read_pos = self.oldest_read_pos();
        let used_space = if write_pos >= oldest_read_pos {
            write_pos - oldest_read_pos
        } else {
            self.size - oldest_read_pos + write_pos
        };
        (used_space as f32) / (self.size as f32)
    }

    /// Get the oldest read position among all subscribers
    pub fn oldest_read_pos(&self) -> usize {
        let subscribers = self.subscribers.lock();
        subscribers
            .iter()
            .map(|sub| sub.lock().read_pos)
            .min()
            .unwrap_or(0)
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        if self.owns_data {
            unsafe {
                let slice = std::slice::from_raw_parts_mut(self.data.get_mut(), self.size);
                drop(Box::from_raw(slice as *mut [u8]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_reader_detects_overwritten_data() {
        let buffer_size = 128;
        let ring_buffer = Arc::new(RingBuffer::new(buffer_size).unwrap());

        // Add a subscriber
        let subscriber = ring_buffer.add_subscriber();

        // Write more data than the buffer can hold to force overwriting
        for i in 0..20 {
            let data = vec![i as u8; 20]; // Each message is 20 bytes + overhead
            ring_buffer.write(&data).unwrap();
        }

        // Reader starts at initial position
        let sub = Arc::clone(&subscriber);
        let ring_buffer_clone = Arc::clone(&ring_buffer);

        let reader = thread::spawn(move || {
            let mut sub = sub.lock();
            loop {
                match sub.read(&ring_buffer_clone) {
                    Ok(data) => {
                        println!("Read data: {:?}", data);
                    }
                    Err(Error::DataOverwritten) => {
                        println!("Data has been overwritten, read_pos adjusted");
                        // Handle data loss
                    }
                    Err(Error::BufferEmpty) => {
                        println!("No more data to read");
                        break;
                    }
                    Err(e) => {
                        panic!("Unexpected error: {:?}", e);
                    }
                }
            }
        });

        reader.join().unwrap();
    }
}
