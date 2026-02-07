/// Core FIR filter implementation shared by bandpass and highpass filters
///
/// Contains the delay line, tap coefficients, and convolution logic.
/// Individual filter types (bandpass, highpass) wrap this and provide
/// their own coefficient design via Parks-McClellan.
pub struct FirFilterCore {
    taps: Vec<f64>,
    delay_line: Vec<f64>,
    pos: usize,
}

impl FirFilterCore {
    /// Create a new FIR filter core with the given tap coefficients
    pub fn new(taps: Vec<f64>) -> Self {
        Self {
            delay_line: vec![0.0; taps.len()],
            taps,
            pos: 0,
        }
    }

    /// Process a single sample through the filter
    pub fn process(&mut self, sample: f32) -> f32 {
        self.delay_line[self.pos] = sample as f64;

        let mut output = 0.0;
        let n = self.taps.len();

        for i in 0..n {
            let delay_idx = (self.pos + n - i) % n;
            output += self.taps[i] * self.delay_line[delay_idx];
        }

        self.pos = (self.pos + 1) % n;
        output as f32
    }

    /// Process an entire buffer of samples in-place
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }

    /// Get the number of taps (filter length)
    pub fn num_taps(&self) -> usize {
        self.taps.len()
    }

    /// Get the group delay in samples (half the filter length for linear phase)
    pub fn group_delay_samples(&self) -> usize {
        (self.taps.len() - 1) / 2
    }

    /// Get access to the tap coefficients
    pub fn taps(&self) -> &[f64] {
        &self.taps
    }
}
