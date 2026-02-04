/// Common trait for audio filters
///
/// Implemented by FirBandpass and FirHighpass.
#[allow(dead_code)]
pub trait Filter {
    /// Process a single sample through the filter
    fn process(&mut self, sample: f32) -> f32;

    /// Process a buffer of samples in-place
    fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }
}
