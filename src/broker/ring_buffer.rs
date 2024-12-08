use crate::error::{Error, Result};
use parking_lot::Mutex;
use std::mem::size_of;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
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
    pub fn get_mut(&self) -> *mut u8 {
        self.ptr
    }

    pub fn get(&self) -> *const u8 {
        self.ptr
    }
}

/// A fixed-size ring buffer implementation using shared memory
pub struct RingBuffer {
    data: SharedPtr,
    size: usize,
    pub read_pos: AtomicUsize,
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

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Relaxed);
            let read_pos = self.read_pos.load(Ordering::Acquire);

            eprintln!(
                "WRITE: write_pos: {}, read_pos: {}, buffer_size: {}",
                write_pos, read_pos, self.size
            );

            // Calculate available space considering wrap-around
            let total_size = size_of::<u32>() + data.len();
            let space_to_end = self.size - write_pos;
            let space_at_start = read_pos;

            // Calculate total available space
            let available_space = if write_pos >= read_pos {
                // Write position is ahead or equal to read position
                // We can use space from write_pos to end, and from start to read_pos
                if space_to_end >= total_size {
                    // We can fit everything without wrapping
                    space_to_end
                } else if space_to_end >= size_of::<u32>() && space_at_start >= data.len() {
                    // We can fit the length prefix at the end and data at the start
                    total_size
                } else if space_at_start >= total_size {
                    // We need to write both length prefix and data at start
                    // First, write a zero-length prefix at the end to mark wrap-around
                    unsafe {
                        let zero_length = 0u32.to_le_bytes();
                        let ptr = self.data.get_mut().add(write_pos);
                        std::ptr::copy_nonoverlapping(zero_length.as_ptr(), ptr, size_of::<u32>());
                    }
                    total_size
                } else {
                    0 // Not enough contiguous space
                }
            } else {
                // Write position has wrapped around
                read_pos - write_pos
            };

            eprintln!(
                "WRITE: Available space: {}, need: {}, total_size: {}",
                available_space, total_size, total_size
            );

            if available_space >= total_size {
                eprintln!("WRITE: Found enough space, writing data");
                // Write length prefix
                let length_bytes = (data.len() as u32).to_le_bytes();
                let mut current_pos = write_pos;

                unsafe {
                    // Write length prefix
                    let mut remaining_length = size_of::<u32>();
                    let mut length_offset = 0;

                    while remaining_length > 0 {
                        let chunk_size = if current_pos + remaining_length > self.size {
                            self.size - current_pos
                        } else {
                            remaining_length
                        };

                        eprintln!(
                            "WRITE: Writing length prefix chunk: {} bytes at position {}",
                            chunk_size, current_pos
                        );

                        let ptr = self.data.get_mut().add(current_pos);
                        std::ptr::copy_nonoverlapping(
                            length_bytes[length_offset..].as_ptr(),
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

                        eprintln!(
                            "WRITE: Writing data chunk: {} bytes at position {}",
                            chunk_size, current_pos
                        );

                        let ptr = self.data.get_mut().add(current_pos);
                        std::ptr::copy_nonoverlapping(
                            data[data_offset..].as_ptr(),
                            ptr,
                            chunk_size,
                        );

                        current_pos = (current_pos + chunk_size) % self.size;
                        remaining_data -= chunk_size;
                        data_offset += chunk_size;
                    }

                    // Update write position
                    std::sync::atomic::fence(Ordering::Release);
                    self.write_pos.store(current_pos, Ordering::Release);
                    eprintln!("WRITE: Updated write_pos to {}", current_pos);
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
    pub fn read(&self) -> Result<Vec<u8>> {
        let start = Instant::now();
        let mut _guard = self.lock.lock();

        while start.elapsed() < MAX_WAIT_TIME {
            let read_pos = self.read_pos.load(Ordering::Relaxed);
            let write_pos = self.write_pos.load(Ordering::Acquire);

            eprintln!(
                "READ: read_pos: {}, write_pos: {}, buffer_size: {}",
                read_pos, write_pos, self.size
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

            if available >= size_of::<u32>() {
                // Read the length prefix
                let mut length_bytes = [0u8; size_of::<u32>()];
                let mut current_pos = read_pos;

                // Read length prefix handling wrap-around
                self.read_exact(&mut length_bytes, current_pos)?;
                current_pos = (current_pos + size_of::<u32>()) % self.size;

                let data_len = u32::from_le_bytes(length_bytes) as usize;
                eprintln!("READ: Message length prefix: {} bytes", data_len);

                // Check if we have enough data
                let total_size = size_of::<u32>() + data_len;
                if available >= total_size {
                    eprintln!(
                        "READ: Have enough data. Total needed: {}, available: {}",
                        total_size, available
                    );

                    let mut data = vec![0u8; data_len];

                    // Read data handling wrap-around
                    self.read_exact(&mut data, current_pos)?;
                    current_pos = (current_pos + data_len) % self.size;

                    // Update read position
                    std::sync::atomic::fence(Ordering::Release);
                    self.read_pos.store(current_pos, Ordering::Release);
                    eprintln!("READ: Updated read_pos to {}", current_pos);
                    return Ok(data);
                } else {
                    eprintln!(
                        "READ: Not enough data. Need: {}, have: {}",
                        total_size, available
                    );
                }
            } else {
                eprintln!("READ: Not enough data for length prefix");
            }

            // Release the lock while waiting
            drop(_guard);
            thread::sleep(Duration::from_micros(100));
            _guard = self.lock.lock();
        }

        eprintln!("READ: Timed out after {:?}", start.elapsed());
        Err(Error::Timeout)
    }

    /// Read a single message from a specific position in the ring buffer
    pub fn read_message(&self, read_pos: usize) -> Result<(Vec<u8>, usize)> {
        let _guard = self.lock.lock();
        let write_pos = self.write_pos.load(Ordering::Acquire);

        eprintln!(
            "READ_MESSAGE: read_pos: {}, write_pos: {}, buffer_size: {}",
            read_pos, write_pos, self.size
        );

        // Check if there is data available
        let available = if write_pos >= read_pos {
            write_pos - read_pos
        } else {
            self.size - read_pos + write_pos
        };

        eprintln!("READ_MESSAGE: Available data: {}", available);

        if available < size_of::<u32>() {
            eprintln!("READ_MESSAGE: Not enough data for length prefix");
            return Err(Error::BufferEmpty); // Not enough data to read length
        }

        // Read length prefix
        let mut length_bytes = [0u8; 4];
        self.read_exact(&mut length_bytes, read_pos)?;
        let msg_len = u32::from_le_bytes(length_bytes) as usize;

        eprintln!("READ_MESSAGE: Message length: {}", msg_len);

        // Check for wrap-around marker (zero length)
        if msg_len == 0 {
            eprintln!("READ_MESSAGE: Found wrap-around marker");
            // Skip the wrap-around marker and start reading from the beginning
            let new_read_pos = 0;
            return self.read_message(new_read_pos);
        }

        let total_size = size_of::<u32>() + msg_len;
        if available < total_size {
            eprintln!(
                "READ_MESSAGE: Not enough data for full message. Need: {}, have: {}",
                total_size, available
            );
            return Err(Error::BufferEmpty); // Not enough data to read the full message
        }

        // Read message data
        let msg_start = (read_pos + size_of::<u32>()) % self.size;
        let mut data = vec![0u8; msg_len];
        self.read_exact(&mut data, msg_start)?;

        let new_read_pos = (read_pos + total_size) % self.size;
        eprintln!(
            "READ_MESSAGE: Successfully read message. New read_pos: {}",
            new_read_pos
        );

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
    fn test_basic_write_read() {
        let buffer = RingBuffer::new(1024).unwrap();
        let data = vec![1, 2, 3, 4, 5];
        buffer.write(&data).unwrap();
        let read_data = buffer.read().unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_multiple_write_read() {
        let buffer = RingBuffer::new(1024).unwrap();
        let data1 = vec![1, 2, 3, 4, 5];
        let data2 = vec![6, 7, 8, 9, 10];

        buffer.write(&data1).unwrap();
        buffer.write(&data2).unwrap();

        let read_data1 = buffer.read().unwrap();
        let read_data2 = buffer.read().unwrap();

        assert_eq!(read_data1, data1);
        assert_eq!(read_data2, data2);
    }

    #[test]
    fn test_wrap_around() {
        let buffer = RingBuffer::new(32).unwrap();
        let data1 = vec![1; 20];
        let data2 = vec![2; 15];

        buffer.write(&data1).unwrap();
        let read_data1 = buffer.read().unwrap();
        buffer.write(&data2).unwrap();
        let read_data2 = buffer.read().unwrap();

        assert_eq!(read_data1, data1);
        assert_eq!(read_data2, data2);
    }

    #[test]
    fn test_concurrent_read_write() {
        let buffer = Arc::new(RingBuffer::new(1024).unwrap());
        let buffer_clone = Arc::clone(&buffer);

        let writer = thread::spawn(move || {
            for i in 0..100 {
                let data = vec![i as u8; 10];
                buffer_clone.write(&data).unwrap();
            }
        });

        let reader = thread::spawn(move || {
            for i in 0..100 {
                let expected = vec![i as u8; 10];
                let data = buffer.read().unwrap();
                assert_eq!(data, expected);
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn test_buffer_full() {
        let buffer = RingBuffer::new(32).unwrap();
        let data = vec![1; 25];

        // First write should succeed
        buffer.write(&data).unwrap();
        let read_data = buffer.read().unwrap();
        assert_eq!(read_data, data);

        // Second write should succeed after read
        buffer.write(&data).unwrap();
        let read_data = buffer.read().unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_large_messages() {
        let buffer = RingBuffer::new(1024).unwrap();
        let data = vec![42; 512];

        for _ in 0..5 {
            buffer.write(&data).unwrap();
            let read_data = buffer.read().unwrap();
            assert_eq!(read_data, data);
        }
    }

    #[test]
    fn test_wrap_around_with_large_messages() {
        let buffer = RingBuffer::new(128).unwrap();
        let data1 = vec![1; 64];
        let data2 = vec![2; 32];
        let data3 = vec![3; 48];

        buffer.write(&data1).unwrap();
        let read_data1 = buffer.read().unwrap();
        assert_eq!(read_data1, data1);

        buffer.write(&data2).unwrap();
        buffer.write(&data3).unwrap();

        let read_data2 = buffer.read().unwrap();
        let read_data3 = buffer.read().unwrap();

        assert_eq!(read_data2, data2);
        assert_eq!(read_data3, data3);
    }

    #[test]
    fn test_concurrent_wrap_around() {
        let buffer = Arc::new(RingBuffer::new(256).unwrap());
        let buffer_clone = Arc::clone(&buffer);

        let writer = thread::spawn(move || {
            for i in 0..50 {
                let data = vec![i as u8; 100];
                match buffer_clone.write(&data) {
                    Ok(()) => (),
                    Err(Error::Timeout) => continue,
                    Err(e) => panic!("Unexpected error: {:?}", e),
                }
            }
        });

        let reader = thread::spawn(move || {
            let mut received = 0;
            while received < 50 {
                match buffer.read() {
                    Ok(data) => {
                        assert_eq!(data.len(), 100);
                        assert!(data.iter().all(|&x| x == data[0]));
                        received += 1;
                    }
                    Err(Error::Timeout) => continue,
                    Err(e) => panic!("Unexpected error: {:?}", e),
                }
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }
}
