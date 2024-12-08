use crate::error::{Error, Result};
use parking_lot::Mutex;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tracing::{debug, error, info};

/// A thread-safe wrapper for raw memory pointer
pub(crate) struct SharedPtr {
    ptr: *mut u8,
}

unsafe impl Send for SharedPtr {}
unsafe impl Sync for SharedPtr {}

impl SharedPtr {
    /// Create a new SharedPtr from a raw pointer
    pub(crate) unsafe fn new(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Get the raw pointer
    pub(crate) unsafe fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

#[repr(C)]
struct RingBufferMetadata {
    write_pos: AtomicUsize,
    sequence: AtomicU64,
    magic: u64,
    version: u32,
    _padding: [u8; 496],
}

const METADATA_MAGIC: u64 = 0x524F4B45525F4D44; // "ROKER_MD" in hex
const METADATA_VERSION: u32 = 1;

pub struct RingBuffer {
    metadata: &'static RingBufferMetadata,
    data: SharedPtr,
    size: usize,
    lock: Mutex<()>,
}

impl RingBuffer {
    pub unsafe fn from_raw_parts(ptr: *mut u8, size: usize, initialize: bool) -> Result<Self> {
        let metadata_ptr = ptr as *mut RingBufferMetadata;

        if initialize {
            debug!(
                "Initializing new ring buffer metadata: size={}, magic=0x{:X}, version={}",
                size, METADATA_MAGIC, METADATA_VERSION
            );

            ptr::write(
                metadata_ptr,
                RingBufferMetadata {
                    write_pos: AtomicUsize::new(0),
                    sequence: AtomicU64::new(0),
                    magic: METADATA_MAGIC,
                    version: METADATA_VERSION,
                    _padding: [0; 496],
                },
            );
        } else {
            let metadata = &*metadata_ptr;
            debug!(
                "Reading existing ring buffer metadata:\n  \
                 Magic: 0x{:X} (expected: 0x{:X})\n  \
                 Version: {} (expected: {})\n  \
                 Write Position: {}\n  \
                 Sequence Number: {}",
                metadata.magic,
                METADATA_MAGIC,
                metadata.version,
                METADATA_VERSION,
                metadata.write_pos.load(Ordering::Relaxed),
                metadata.sequence.load(Ordering::Relaxed)
            );

            if metadata.magic != METADATA_MAGIC {
                error!("Invalid metadata magic number: 0x{:X}", metadata.magic);
                return Err(Error::InvalidConfig("Invalid metadata magic number".into()));
            }
            if metadata.version != METADATA_VERSION {
                error!(
                    "Incompatible metadata version: {} (expected {})",
                    metadata.version, METADATA_VERSION
                );
                return Err(Error::InvalidConfig("Incompatible metadata version".into()));
            }
        }

        let data_ptr = ptr.add(size_of::<RingBufferMetadata>());
        debug!(
            "Ring buffer layout:\n  \
             Metadata size: {} bytes\n  \
             Data offset: {} bytes\n  \
             Data size: {} bytes",
            size_of::<RingBufferMetadata>(),
            size_of::<RingBufferMetadata>(),
            size
        );

        Ok(Self {
            metadata: &*metadata_ptr,
            data: SharedPtr::new(data_ptr),
            size,
            lock: Mutex::new(()),
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let _guard = self.lock.lock();

        let write_pos = self.metadata.write_pos.load(Ordering::Relaxed);
        let sequence = self.metadata.sequence.fetch_add(1, Ordering::Relaxed);

        let mut current_pos = write_pos;

        // Write sequence number
        let seq_bytes = sequence.to_le_bytes();
        unsafe {
            self.write_bytes(&seq_bytes, &mut current_pos)?;
        }

        // Write length and data
        let len_bytes = (data.len() as u32).to_le_bytes();
        unsafe {
            self.write_bytes(&len_bytes, &mut current_pos)?;
            self.write_bytes(data, &mut current_pos)?;
        }

        // Update write position
        self.metadata
            .write_pos
            .store(current_pos, Ordering::Release);

        Ok(())
    }

    unsafe fn write_bytes(&self, src: &[u8], current_pos: &mut usize) -> Result<()> {
        let dst = self.data.as_ptr().add(*current_pos);
        ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        *current_pos = (*current_pos + src.len()) % self.size;
        Ok(())
    }

    fn read_bytes(&self, dst: &mut [u8], pos: usize) -> Result<()> {
        unsafe {
            let src = self.data.as_ptr().add(pos);
            ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), dst.len());
        }
        Ok(())
    }

    pub fn write_pos(&self) -> usize {
        self.metadata.write_pos.load(Ordering::Acquire)
    }

    pub fn oldest_read_pos(&self) -> usize {
        // For now, just return 0 as we're not tracking individual read positions
        0
    }

    pub fn read_message(&self, read_pos: usize) -> Result<(Vec<u8>, usize)> {
        let write_pos = self.metadata.write_pos.load(Ordering::Acquire);

        if read_pos == write_pos {
            debug!("Buffer is empty (read_pos == write_pos)");
            return Err(Error::BufferEmpty);
        }

        let mut current_pos = read_pos;
        debug!("Starting read_message from position: {}", current_pos);

        // Read sequence number
        let mut seq_bytes = [0u8; 8];
        self.read_bytes(&mut seq_bytes, current_pos)?;
        let sequence = u64::from_le_bytes(seq_bytes);
        current_pos = (current_pos + size_of::<u64>()) % self.size;
        debug!(
            "Read sequence number: {}, new pos: {}",
            sequence, current_pos
        );

        // Read length prefix
        let mut length_bytes = [0u8; 4];
        self.read_bytes(&mut length_bytes, current_pos)?;
        let data_len = u32::from_le_bytes(length_bytes) as usize;
        current_pos = (current_pos + size_of::<u32>()) % self.size;

        if data_len == 0 || data_len > self.size {
            error!(
                "Invalid data length: {} (buffer size: {})",
                data_len, self.size
            );
            return Err(Error::InvalidConfig("Invalid message size".into()));
        }

        // Read data
        let mut data = vec![0u8; data_len];
        self.read_bytes(&mut data, current_pos)?;
        current_pos = (current_pos + data_len) % self.size;
        debug!(
            "Read {} bytes of data, final pos: {}",
            data_len, current_pos
        );

        Ok((data, current_pos))
    }

    pub fn usage(&self) -> f32 {
        let write_pos = self.metadata.write_pos.load(Ordering::Relaxed);
        let oldest_read = self.oldest_read_pos();

        if write_pos >= oldest_read {
            (write_pos - oldest_read) as f32 / self.size as f32
        } else {
            (self.size - (oldest_read - write_pos)) as f32 / self.size as f32
        }
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        // No need to explicitly clean up metadata as it's in shared memory
    }
}
