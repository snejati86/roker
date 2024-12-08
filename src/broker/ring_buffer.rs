use crate::error::{Error, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
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

    /// Get the size of the ring buffer
    pub fn size(&self) -> usize {
        self.size
    }

    /// Write data to the ring buffer with timeout
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let start = Instant::now();
        let _guard = self.lock.lock();

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Relaxed);
            let read_pos = self.read_pos.load(Ordering::Relaxed);

            let available = if write_pos >= read_pos {
                if read_pos == 0 {
                    self.size - write_pos - 1
                } else {
                    self.size - write_pos + read_pos - 1
                }
            } else {
                read_pos - write_pos - 1
            };

            if data.len() <= available {
                unsafe {
                    let write_end = write_pos + data.len();
                    if write_end > self.size {
                        // Write needs to wrap around
                        let first_chunk = self.size - write_pos;
                        let ptr = self.data.get_mut().add(write_pos);
                        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, first_chunk);

                        let ptr = self.data.get_mut();
                        std::ptr::copy_nonoverlapping(
                            data[first_chunk..].as_ptr(),
                            ptr,
                            data.len() - first_chunk,
                        );
                    } else {
                        let ptr = self.data.get_mut().add(write_pos);
                        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                    }
                }

                self.write_pos
                    .store((write_pos + data.len()) % self.size, Ordering::Release);
                return Ok(());
            }

            std::thread::sleep(Duration::from_millis(1));
        }

        Err(Error::Timeout)
    }

    /// Read data from the ring buffer with timeout
    pub fn read(&self, buf: &mut [u8]) -> Result<()> {
        let start = Instant::now();
        let _guard = self.lock.lock();

        while start.elapsed() < MAX_WAIT_TIME {
            let write_pos = self.write_pos.load(Ordering::Acquire);
            let read_pos = self.read_pos.load(Ordering::Relaxed);

            let available = if write_pos >= read_pos {
                write_pos - read_pos
            } else {
                self.size - read_pos + write_pos
            };

            if available > 0 {
                let to_read = buf.len().min(available);

                unsafe {
                    let read_end = read_pos + to_read;
                    if read_end > self.size {
                        // Read needs to wrap around
                        let first_chunk = self.size - read_pos;
                        let ptr = self.data.get_mut().add(read_pos);
                        std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), first_chunk);

                        let ptr = self.data.get_mut();
                        std::ptr::copy_nonoverlapping(
                            ptr,
                            buf[first_chunk..].as_mut_ptr(),
                            to_read - first_chunk,
                        );
                    } else {
                        let ptr = self.data.get_mut().add(read_pos);
                        std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), to_read);
                    }
                }

                self.read_pos
                    .store((read_pos + to_read) % self.size, Ordering::Release);
                return Ok(());
            }

            std::thread::sleep(Duration::from_millis(1));
        }

        Err(Error::Timeout)
    }

    /// Get the current buffer usage
    pub fn usage(&self) -> f32 {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Relaxed);

        let used = if write_pos >= read_pos {
            write_pos - read_pos
        } else {
            self.size - read_pos + write_pos
        };

        used as f32 / self.size as f32
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        if self.owns_data {
            unsafe {
                let ptr = self.data.get_mut();
                if !ptr.is_null() {
                    // Reconstruct the original Box<[u8]> using a pointer to the slice
                    let slice_ptr = std::ptr::slice_from_raw_parts_mut(ptr, self.size);
                    drop(Box::from_raw(slice_ptr));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_creation() {
        let buffer = RingBuffer::new(1024).unwrap();
        assert_eq!(buffer.size(), 1024);
    }

    #[test]
    fn test_invalid_buffer_size() {
        assert!(RingBuffer::new(0).is_err());
        assert!(RingBuffer::new(1023).is_err()); // Not power of 2
    }

    #[test]
    fn test_buffer_write_read() {
        let buffer = RingBuffer::new(1024).unwrap();
        let data = b"Hello, World!";
        buffer.write(data).unwrap();

        let mut read_buf = vec![0u8; data.len()];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(&read_buf, data);
    }

    #[test]
    fn test_buffer_full() {
        let buffer = RingBuffer::new(16).unwrap();
        let data = vec![1u8; 14]; // Leave two bytes for internal bookkeeping
        buffer.write(&data).unwrap();
        assert!(buffer.write(&[1u8, 1u8]).is_err());
    }

    #[test]
    fn test_buffer_empty() {
        let buffer = RingBuffer::new(16).unwrap();
        let mut buf = [0u8; 1];
        assert!(buffer.read(&mut buf).is_err());
    }

    #[test]
    fn test_buffer_wraparound() {
        let buffer = RingBuffer::new(16).unwrap();
        let data1 = vec![1u8; 10];
        let data2 = vec![2u8; 4];
        let data3 = vec![3u8; 4];

        // Fill buffer
        buffer.write(&data1).unwrap();
        buffer.write(&data2).unwrap();

        // Read first chunk
        let mut read_buf = vec![0u8; 10];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(&read_buf, &data1);

        // Write more data that wraps around
        buffer.write(&data3).unwrap();

        // Read remaining data
        let mut read_buf = vec![0u8; 4];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(&read_buf, &data2);

        let mut read_buf = vec![0u8; 4];
        buffer.read(&mut read_buf).unwrap();
        assert_eq!(&read_buf, &data3);
    }
}
