use crate::broker::RingBuffer;
use crate::error::Result;
use std::sync::Arc;

pub struct Publisher {
    buffer: Arc<RingBuffer>,
}

impl Publisher {
    pub fn new(buffer: Arc<RingBuffer>) -> Self {
        Self { buffer }
    }

    pub fn publish(&self, data: &[u8]) -> Result<()> {
        self.buffer.write(data)
    }
} 