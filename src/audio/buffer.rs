/// Stereo sample
#[derive(Copy, Clone, Debug, Default)]
pub struct StereoSample {
    pub left: f32,
    pub right: f32,
}

/// Ring buffer for audio samples
pub struct AudioRingBuffer {
    buffer: Vec<StereoSample>,
    capacity: usize,
}

impl AudioRingBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(8192),
            capacity: 8192,
        }
    }

    /// Push interleaved stereo samples [L, R, L, R, ...]
    pub fn push_interleaved(&mut self, data: &[f32]) {
        for chunk in data.chunks_exact(2) {
            let sample = StereoSample {
                left: chunk[0],
                right: chunk[1],
            };
            self.buffer.push(sample);
        }

        // Keep only the most recent samples
        if self.buffer.len() > self.capacity {
            let excess = self.buffer.len() - self.capacity;
            self.buffer.drain(0..excess);
        }
    }

    /// Get latest N samples in chronological order (oldest to newest)
    pub fn latest(&self, count: usize) -> Vec<StereoSample> {
        let len = self.buffer.len().min(count);
        if len == 0 {
            return Vec::new();
        }

        let start = self.buffer.len() - len;
        self.buffer[start..].to_vec()
    }

    /// Check buffer length
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if buffer is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl Default for AudioRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}
