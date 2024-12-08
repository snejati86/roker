use crate::error::{Error, Result};
use parking_lot::Mutex;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
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
    unsafe fn get_mut(&self) -> *mut u8 {
        self.ptr
    }
}

/// A fixed-size ring buffer implementation using shared memory
pub struct RingBuffer {
    data: SharedPtr,
    size: usize,
    read_pos: AtomicUsize,
    write_pos: AtomicUsize,
    lock: Mutex<()>,
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
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
            lock: Mutex::new(()),
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
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
            lock: Mutex::new(()),
            owns_data: owns_memory,
        })
    }

    /// Write data to the ring buffer with timeout
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let start = Instant::now();
        let mut _guard = self.lock.lock();

        // Calculate total size needed: length prefix + data
        let total_size = size_of::<u32>() + data.len();
        eprintln!(
            "Attempting to write {} bytes (total_size: {})",
            data.len(),
            total_size
        );

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Relaxed);
            let read_pos = self.read_pos.load(Ordering::Acquire);
            eprintln!(
                "write_pos: {}, read_pos: {}, buffer_size: {}",
                write_pos, read_pos, self.size
            );

            // Calculate available space considering wrap-around
            let available = if write_pos >= read_pos {
                // Write position is ahead or equal to read position
                // Available space is from write_pos to end + from start to read_pos
                if read_pos == 0 {
                    // Special case: read_pos at start, need to leave one byte
                    self.size - write_pos - 1
                } else {
                    // Available space is from write_pos to end + from start to read_pos - 1
                    self.size - write_pos + read_pos - 1
                }
            } else {
                // Write position has wrapped around
                // Available space is directly between positions
                read_pos - write_pos - 1
            };

            eprintln!("Available space: {}, need: {}", available, total_size);

            if total_size <= available {
                eprintln!("Writing data at position {}", write_pos);
                // Write the length prefix and data
                unsafe {
                    let length_bytes = (data.len() as u32).to_le_bytes();

                    // Write length prefix
                    let mut current_pos = write_pos;
                    let mut remaining_length = size_of::<u32>();
                    let mut length_offset = 0;

                    while remaining_length > 0 {
                        let chunk_size = if current_pos + remaining_length > self.size {
                            self.size - current_pos
                        } else {
                            remaining_length
                        };

                        let ptr = self.data.get_mut().add(current_pos);
                        std::ptr::copy_nonoverlapping(
                            length_bytes[length_offset..length_offset + chunk_size].as_ptr(),
                            ptr,
                            chunk_size,
                        );

                        current_pos = (current_pos + chunk_size) % self.size;
                        remaining_length -= chunk_size;
                        length_offset += chunk_size;
                    }

                    // Write data
                    let mut remaining_data = data.len();
                    let mut data_offset = 0;

                    while remaining_data > 0 {
                        let chunk_size = if current_pos + remaining_data > self.size {
                            self.size - current_pos
                        } else {
                            remaining_data
                        };

                        let ptr = self.data.get_mut().add(current_pos);
                        std::ptr::copy_nonoverlapping(
                            data[data_offset..data_offset + chunk_size].as_ptr(),
                            ptr,
                            chunk_size,
                        );

                        current_pos = (current_pos + chunk_size) % self.size;
                        remaining_data -= chunk_size;
                        data_offset += chunk_size;
                    }

                    // Update write position
                    self.write_pos.store(current_pos, Ordering::Release);
                    eprintln!("Write completed, new write_pos: {}", current_pos);
                }

                return Ok(());
            }

            // Release the lock while waiting
            drop(_guard);
            thread::sleep(Duration::from_micros(100));
            _guard = self.lock.lock();
        }

        eprintln!("Write timed out after {:?}", start.elapsed());
        Err(Error::Timeout)
    }

    #[cfg(test)]
    /// Read data from the ring buffer with timeout
    pub fn read(&self, buf: &mut [u8]) -> Result<()> {
        let start = Instant::now();
        let mut _guard = self.lock.lock();

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Acquire);
            let read_pos = self.read_pos.load(Ordering::Relaxed);
            eprintln!(
                "READ: write_pos: {}, read_pos: {}, buffer_size: {}",
                write_pos, read_pos, self.size
            );

            // Calculate available data considering wrap-around
            let available = if write_pos >= read_pos {
                write_pos - read_pos
            } else {
                self.size - read_pos + write_pos
            };

            eprintln!(
                "READ: Available data: {}, need length prefix: {}",
                available,
                size_of::<u32>()
            );

            // Check if there's any data to read
            if available == 0 {
                eprintln!("READ: No data available, waiting...");
                // Release the lock while waiting
                drop(_guard);
                thread::sleep(Duration::from_millis(1));
                _guard = self.lock.lock();
                continue;
            }

            if available >= size_of::<u32>() {
                // Read the length prefix
                let mut length_bytes = [0u8; size_of::<u32>()];
                unsafe {
                    let read_end = read_pos + size_of::<u32>();
                    if read_end > self.size {
                        // Length prefix wraps around
                        let first_chunk = self.size - read_pos;
                        eprintln!(
                            "READ: Length prefix wraps around. First chunk: {}, remaining: {}",
                            first_chunk,
                            size_of::<u32>() - first_chunk
                        );
                        let ptr = self.data.get_mut().add(read_pos);
                        std::ptr::copy_nonoverlapping(ptr, length_bytes.as_mut_ptr(), first_chunk);

                        let ptr = self.data.get_mut();
                        std::ptr::copy_nonoverlapping(
                            ptr,
                            length_bytes[first_chunk..].as_mut_ptr(),
                            size_of::<u32>() - first_chunk,
                        );
                    } else {
                        let ptr = self.data.get_mut().add(read_pos);
                        std::ptr::copy_nonoverlapping(
                            ptr,
                            length_bytes.as_mut_ptr(),
                            size_of::<u32>(),
                        );
                    }
                }

                let data_len = u32::from_le_bytes(length_bytes) as usize;
                eprintln!("READ: Message length: {}", data_len);

                if data_len > buf.len() {
                    eprintln!(
                        "READ: Buffer too small. Need: {}, have: {}",
                        data_len,
                        buf.len()
                    );
                    return Err(Error::BufferTooSmall);
                }

                // Check if we have enough data for the entire message
                if available < size_of::<u32>() + data_len {
                    eprintln!(
                        "READ: Not enough data for complete message. Available: {}, need: {}",
                        available,
                        size_of::<u32>() + data_len
                    );
                    // Release the lock while waiting
                    drop(_guard);
                    thread::sleep(Duration::from_millis(1));
                    _guard = self.lock.lock();
                    continue;
                }

                // Update read position after reading length
                let new_read_pos = (read_pos + size_of::<u32>()) % self.size;
                self.read_pos.store(new_read_pos, Ordering::Release);
                eprintln!("READ: Updated read_pos after length: {}", new_read_pos);

                unsafe {
                    let read_end = new_read_pos + data_len;
                    if read_end > self.size {
                        // Data needs to wrap around
                        let first_chunk = self.size - new_read_pos;
                        eprintln!(
                            "READ: Message wraps around. First chunk: {}, remaining: {}",
                            first_chunk,
                            data_len - first_chunk
                        );
                        let ptr = self.data.get_mut().add(new_read_pos);
                        std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), first_chunk);

                        let ptr = self.data.get_mut();
                        std::ptr::copy_nonoverlapping(
                            ptr,
                            buf[first_chunk..data_len].as_mut_ptr(),
                            data_len - first_chunk,
                        );

                        // Update read position to wrapped position
                        self.read_pos
                            .store(data_len - first_chunk, Ordering::Release);
                        eprintln!(
                            "READ: Read wrapped around, new read_pos: {}",
                            data_len - first_chunk
                        );
                    } else {
                        let ptr = self.data.get_mut().add(new_read_pos);
                        std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), data_len);

                        // Update read position normally
                        let final_read_pos = read_end % self.size;
                        self.read_pos.store(final_read_pos, Ordering::Release);
                        eprintln!(
                            "READ: Read completed normally, new read_pos: {}",
                            final_read_pos
                        );
                    }
                }

                return Ok(());
            }

            eprintln!("READ: Waiting for more data...");
            // Release the lock while waiting
            drop(_guard);
            thread::sleep(Duration::from_millis(1));
            _guard = self.lock.lock();
        }

        eprintln!("READ: Timed out after {:?}", start.elapsed());
        Err(Error::Timeout)
    }

    /// Read a single message from a specific position in the ring buffer
    pub fn read_message(&self, read_pos: usize) -> Result<(Vec<u8>, usize)> {
        let _guard = self.lock.lock();
        let write_pos = self.write_pos.load(Ordering::Acquire);

        // Check if there is data available
        let available = if write_pos >= read_pos {
            write_pos - read_pos
        } else {
            self.size - read_pos + write_pos
        };

        if available < size_of::<u32>() {
            return Err(Error::BufferEmpty); // Not enough data to read length
        }

        // Read length prefix
        let mut length_bytes = [0u8; 4];
        self.read_exact(&mut length_bytes, read_pos)?;
        let msg_len = u32::from_le_bytes(length_bytes) as usize;

        let total_size = size_of::<u32>() + msg_len;
        if available < total_size {
            return Err(Error::BufferEmpty); // Not enough data to read the full message
        }

        // Read message data
        let msg_start = (read_pos + size_of::<u32>()) % self.size;
        let mut data = vec![0u8; msg_len];
        self.read_exact(&mut data, msg_start)?;

        let new_read_pos = (read_pos + total_size) % self.size;

        Ok((data, new_read_pos))
    }

    /// Helper method to read an exact number of bytes handling wrapping
    fn read_exact(&self, buf: &mut [u8], read_pos: usize) -> Result<()> {
        let read_end = read_pos + buf.len();

        unsafe {
            if read_end > self.size {
                // Read needs to wrap around
                let first_chunk = self.size - read_pos;
                let ptr = self.data.get_mut().add(read_pos);
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), first_chunk);

                let ptr = self.data.get_mut();
                std::ptr::copy_nonoverlapping(
                    ptr,
                    buf[first_chunk..].as_mut_ptr(),
                    buf.len() - first_chunk,
                );
            } else {
                let ptr = self.data.get_mut().add(read_pos);
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), buf.len());
            }
        }

        Ok(())
    }

    /// Calculate the current buffer usage as a percentage
    pub fn usage(&self) -> f32 {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Relaxed);

        let used_space = if write_pos >= read_pos {
            write_pos - read_pos
        } else {
            self.size - read_pos + write_pos
        };

        used_space as f32 / self.size as f32
    }

    /// Get the current write position
    pub fn write_pos(&self) -> usize {
        self.write_pos.load(Ordering::Acquire)
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

    #[test]
    fn test_write_read_basic() {
        let buffer = RingBuffer::new(64).unwrap();
        let data = vec![1, 2, 3, 4];
        let mut read_buf = vec![0; 4];

        buffer.write(&data).unwrap();
        buffer.read(&mut read_buf).unwrap();

        assert_eq!(read_buf, data);
    }

    #[test]
    fn test_write_read_multiple() {
        let buffer = RingBuffer::new(64).unwrap();
        let data1 = vec![1, 2, 3, 4];
        let data2 = vec![5, 6, 7, 8];
        let mut read_buf = vec![0; 4];

        buffer.write(&data1).unwrap();
        buffer.write(&data2).unwrap();

        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data1);

        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data2);
    }

    #[test]
    fn test_write_wrap_around() {
        // Create a buffer just big enough for two messages plus their length prefixes
        // Each message needs: 4 bytes (length) + data.len() bytes
        let buffer_size = 64; // Must be power of 2
        let buffer = RingBuffer::new(buffer_size).unwrap();

        // Fill most of the buffer to force wrap-around
        let data1 = vec![1; 20]; // First message will take 24 bytes (4 + 20)
        buffer.write(&data1).unwrap();

        // This write should wrap around
        let data2 = vec![2; 4];
        buffer.write(&data2).unwrap();

        // Read both messages
        let mut read_buf = vec![0; 20];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data1);

        let mut read_buf = vec![0; 4];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data2);
    }

    #[test]
    fn test_read_wrap_around() {
        let buffer_size = 64;
        let buffer = RingBuffer::new(buffer_size).unwrap();

        // Write two messages
        let data1 = vec![1; 20];
        let data2 = vec![2; 20];
        buffer.write(&data1).unwrap();
        buffer.write(&data2).unwrap();

        // Read the first message
        let mut read_buf = vec![0; 20];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data1);

        // Now read the second message, which should wrap around
        let mut read_buf = vec![0; 20];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data2);
    }

    #[test]
    fn test_buffer_full() {
        let buffer_size = 32;
        let buffer = RingBuffer::new(buffer_size).unwrap();

        // Try to write more data than the buffer can hold
        let data = vec![1; buffer_size];
        assert!(buffer.write(&data).is_err());
    }

    #[test]
    fn test_write_read_exact_size() {
        let buffer_size = 32;
        let buffer = RingBuffer::new(buffer_size).unwrap();

        // Write a message that exactly fits (including length prefix)
        let data = vec![1; buffer_size - size_of::<u32>() - 1];
        buffer.write(&data).unwrap();

        let mut read_buf = vec![0; buffer_size - size_of::<u32>() - 1];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, data);
    }

    #[test]
    fn test_multiple_wrap_around() {
        let buffer_size = 64;
        let buffer = RingBuffer::new(buffer_size).unwrap();
        let mut read_buf = vec![0; 4];

        // Write and read multiple times to force multiple wrap-arounds
        for i in 0..20 {
            let data = vec![i as u8; 4];
            buffer.write(&data).unwrap();
            buffer.read(&mut read_buf).unwrap();
            assert_eq!(read_buf, data);
        }
    }

    #[test]
    fn test_concurrent_read_write() {
        use std::sync::Arc;
        use std::thread;

        let buffer_size = 256;
        let buffer = Arc::new(RingBuffer::new(buffer_size).unwrap());
        let buffer_clone = Arc::clone(&buffer);

        let writer = thread::spawn(move || {
            for i in 0..100 {
                let data = vec![i as u8; 4];
                buffer_clone.write(&data).unwrap();
            }
        });

        let reader = thread::spawn(move || {
            let mut read_buf = vec![0; 4];
            for i in 0..100 {
                buffer.read(&mut read_buf).unwrap();
                assert_eq!(read_buf, vec![i as u8; 4]);
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }
}
