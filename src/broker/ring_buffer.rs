use crate::error::{Error, Result};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, trace, warn};

#[repr(C)]
pub(crate) struct RingBuffer {
    data: *mut u8,
    size: usize,
    read_pos: AtomicUsize,
    write_pos: AtomicUsize,
    lock: RwLock<()>,
}

// SAFETY: The RingBuffer uses atomic operations and proper locking
// to ensure thread safety of all operations.
unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    pub fn new(size: usize) -> Result<Self> {
        if !size.is_power_of_two() {
            return Err(Error::InvalidBufferSize);
        }

        let data = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };

        if data == libc::MAP_FAILED {
            return Err(Error::Io(std::io::Error::last_os_error()));
        }

        debug!("Created new ring buffer with size: {}", size);

        Ok(Self {
            data: data as *mut u8,
            size,
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
            lock: RwLock::new(()),
        })
    }

    /// Creates a RingBuffer from existing shared memory
    pub unsafe fn from_raw_parts(ptr: *mut u8, size: usize) -> Result<Self> {
        if !size.is_power_of_two() {
            return Err(Error::InvalidBufferSize);
        }

        debug!("Creating ring buffer from raw parts, size: {}", size);

        Ok(Self {
            data: ptr,
            size,
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
            lock: RwLock::new(()),
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let _lock = self.lock.write();
        
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        let available_space = if write_pos >= read_pos {
            if read_pos == 0 {
                self.size - write_pos
            } else {
                self.size - write_pos + read_pos - 1
            }
        } else {
            read_pos - write_pos - 1
        };

        if data.len() > available_space {
            warn!("Buffer full: need {} bytes, only {} available", data.len(), available_space);
            return Err(Error::BufferFull);
        }

        trace!("Writing {} bytes at position {}", data.len(), write_pos);

        let write_end = write_pos + data.len();
        let wrap_around = write_end > self.size;

        unsafe {
            if wrap_around {
                let first_chunk_size = self.size - write_pos;
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    self.data.add(write_pos),
                    first_chunk_size,
                );
                std::ptr::copy_nonoverlapping(
                    data[first_chunk_size..].as_ptr(),
                    self.data,
                    data.len() - first_chunk_size,
                );
            } else {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    self.data.add(write_pos),
                    data.len(),
                );
            }
        }

        self.write_pos.store(
            (write_pos + data.len()) % self.size,
            Ordering::Release,
        );

        Ok(())
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<()> {
        let _lock = self.lock.read();
        
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let write_pos = self.write_pos.load(Ordering::Acquire);
        
        let available_data = if write_pos >= read_pos {
            write_pos - read_pos
        } else {
            self.size - read_pos + write_pos
        };

        if available_data == 0 {
            return Err(Error::BufferEmpty);
        }

        if buf.len() > available_data {
            return Err(Error::BufferEmpty);
        }

        trace!("Reading {} bytes from position {}", buf.len(), read_pos);

        let read_end = read_pos + buf.len();
        let wrap_around = read_end > self.size;

        unsafe {
            if wrap_around {
                let first_chunk_size = self.size - read_pos;
                std::ptr::copy_nonoverlapping(
                    self.data.add(read_pos),
                    buf.as_mut_ptr(),
                    first_chunk_size,
                );
                std::ptr::copy_nonoverlapping(
                    self.data,
                    buf[first_chunk_size..].as_mut_ptr(),
                    buf.len() - first_chunk_size,
                );
            } else {
                std::ptr::copy_nonoverlapping(
                    self.data.add(read_pos),
                    buf.as_mut_ptr(),
                    buf.len(),
                );
            }
        }

        self.read_pos.store(
            (read_pos + buf.len()) % self.size,
            Ordering::Release,
        );

        Ok(())
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn usage(&self) -> f32 {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        
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
        debug!("Dropping ring buffer of size {}", self.size);
        unsafe {
            libc::munmap(self.data as *mut libc::c_void, self.size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_ring_buffer_creation() {
        let buffer = RingBuffer::new(4096).expect("Failed to create buffer");
        assert_eq!(buffer.size, 4096);
        assert_eq!(buffer.read_pos.load(Ordering::Relaxed), 0);
        assert_eq!(buffer.write_pos.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_invalid_buffer_size() {
        let result = RingBuffer::new(1000);
        assert!(matches!(result, Err(Error::InvalidBufferSize)));
    }

    #[test]
    fn test_buffer_write_read() {
        let buffer = RingBuffer::new(4096).expect("Failed to create buffer");
        let test_data = b"Hello, World!";
        
        buffer.write(test_data).expect("Failed to write data");
        
        let mut read_buf = vec![0u8; test_data.len()];
        buffer.read(&mut read_buf).expect("Failed to read data");
        
        assert_eq!(&read_buf, test_data);
    }

    #[test]
    fn test_buffer_wraparound() {
        let buffer = RingBuffer::new(16).expect("Failed to create buffer");
        
        buffer.write(b"1234567890").expect("Failed to write first batch");
        
        let mut read_buf = vec![0u8; 10];
        buffer.read(&mut read_buf).expect("Failed to read first batch");
        assert_eq!(&read_buf, b"1234567890");
        
        buffer.write(b"abcdef").expect("Failed to write second batch");
        
        let mut read_buf = vec![0u8; 6];
        buffer.read(&mut read_buf).expect("Failed to read second batch");
        assert_eq!(&read_buf, b"abcdef");
    }

    #[test]
    fn test_buffer_full() {
        let buffer = RingBuffer::new(16).expect("Failed to create buffer");
        
        buffer.write(b"123456789012345").expect("Failed to write data");
        
        let result = buffer.write(b"overflow");
        assert!(matches!(result, Err(Error::BufferFull)));
    }

    #[test]
    fn test_buffer_empty() {
        let buffer = RingBuffer::new(16).expect("Failed to create buffer");
        let mut read_buf = vec![0u8; 8];
        
        let result = buffer.read(&mut read_buf);
        assert!(matches!(result, Err(Error::BufferEmpty)));
    }

    #[test]
    fn test_concurrent_access() {
        let buffer = Arc::new(RingBuffer::new(4096).expect("Failed to create buffer"));
        let reader_buffer = Arc::clone(&buffer);
        let writer_buffer = Arc::clone(&buffer);
        
        let writer = thread::spawn(move || {
            for i in 0..10 {
                let data = format!("Message {}", i);
                while writer_buffer.write(data.as_bytes()).is_err() {
                    thread::yield_now();
                }
            }
        });
        
        let reader = thread::spawn(move || {
            let mut messages = Vec::new();
            let mut buf = vec![0u8; 20];
            for _ in 0..10 {
                while reader_buffer.read(&mut buf).is_err() {
                    thread::yield_now();
                }
                messages.push(String::from_utf8_lossy(&buf).to_string());
            }
            messages
        });
        
        writer.join().expect("Writer thread panicked");
        let received = reader.join().expect("Reader thread panicked");
        
        assert_eq!(received.len(), 10);
    }
} 