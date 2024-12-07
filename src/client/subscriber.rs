use crate::broker::RingBuffer;
use crate::error::Result;
use std::sync::Arc;

pub struct Subscriber {
    buffer: Arc<RingBuffer>,
}

impl Subscriber {
    pub fn new(buffer: Arc<RingBuffer>) -> Self {
        Self { buffer }
    }

    pub fn receive(&self, buf: &mut [u8]) -> Result<()> {
        self.buffer.read(buf)
    }
} 