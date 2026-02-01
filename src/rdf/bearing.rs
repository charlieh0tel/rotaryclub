use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{
    AutomaticGainControl, BandpassFilter, MovingAverage, ZeroCrossingDetector, phase_to_bearing,
};
use std::f32::consts::PI;

pub struct ZeroCrossingBearingCalculator {
    agc: AutomaticGainControl,
    bandpass: BandpassFilter,
    zero_detector: ZeroCrossingDetector,
    sample_counter: usize,
    bearing_smoother: MovingAverage,
}

impl ZeroCrossingBearingCalculator {
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate as u32),
            bandpass: BandpassFilter::new(
                doppler_config.bandpass_low,
                doppler_config.bandpass_high,
                sample_rate,
                doppler_config.filter_order,
            )?,
            zero_detector: ZeroCrossingDetector::new(doppler_config.zero_cross_hysteresis),
            sample_counter: 0,
            bearing_smoother: MovingAverage::new(smoothing),
        })
    }

    /// Process doppler channel and calculate bearing relative to north tick
    pub fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        // Apply AGC to normalize signal amplitude
        let mut normalized = doppler_buffer.to_vec();
        self.agc.process_buffer(&mut normalized);

        // Filter doppler tone
        let mut filtered = normalized;
        self.bandpass.process_buffer(&mut filtered);

        // Find zero crossings
        let crossings = self.zero_detector.find_all_crossings(&filtered);

        eprintln!("BearingCalc: sample_counter={}, tick.sample_index={}, buffer_len={}, crossings={}",
                  self.sample_counter, north_tick.sample_index, doppler_buffer.len(), crossings.len());

        if crossings.is_empty() {
            eprintln!("BearingCalc: No zero crossings found");
            self.sample_counter += doppler_buffer.len();
            return None;
        }

        // Get rotation period
        let samples_per_rotation = north_tick.period?;

        // Use the first crossing in the buffer
        let crossing_idx = crossings[0];
        let global_crossing = self.sample_counter + crossing_idx;

        eprintln!("BearingCalc: crossing_idx={}, global_crossing={}, samples_per_rotation={}",
                  crossing_idx, global_crossing, samples_per_rotation);

        // Calculate samples elapsed since north tick
        let samples_since_tick = if global_crossing >= north_tick.sample_index {
            (global_crossing - north_tick.sample_index) as f32
        } else {
            // Handle wrap-around (shouldn't normally happen)
            eprintln!("BearingCalc: Wrap-around detected! global_crossing < tick.sample_index");
            self.sample_counter += doppler_buffer.len();
            return None;
        };

        // Calculate phase in radians
        let phase = (samples_since_tick / samples_per_rotation) * 2.0 * PI;

        // Convert to bearing (0-360 degrees)
        let raw_bearing = phase_to_bearing(phase);

        // Apply smoothing
        let smoothed_bearing = self.bearing_smoother.add(raw_bearing);

        self.sample_counter += doppler_buffer.len();

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: self.calculate_confidence(&crossings),
            timestamp_samples: global_crossing,
        })
    }

    /// Calculate confidence metric based on signal quality
    fn calculate_confidence(&self, crossings: &[usize]) -> f32 {
        // Simple confidence: more crossings = better signal
        // In a real implementation, could use SNR, coherence, etc.
        let crossing_rate = crossings.len() as f32;
        if crossing_rate > 0.0 {
            (crossing_rate / 10.0).min(1.0)
        } else {
            0.0
        }
    }

    /// Reset calculator state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.sample_counter = 0;
        self.zero_detector.reset();
        self.bearing_smoother.reset();
    }
}

/// Correlation-based bearing calculator using I/Q demodulation
pub struct CorrelationBearingCalculator {
    agc: AutomaticGainControl,
    bandpass: BandpassFilter,
    sample_counter: usize,
    bearing_smoother: MovingAverage,
    sample_rate: f32,
}

impl CorrelationBearingCalculator {
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate as u32),
            bandpass: BandpassFilter::new(
                doppler_config.bandpass_low,
                doppler_config.bandpass_high,
                sample_rate,
                doppler_config.filter_order,
            )?,
            sample_counter: 0,
            bearing_smoother: MovingAverage::new(smoothing),
            sample_rate,
        })
    }

    /// Process doppler channel using I/Q correlation to extract phase
    pub fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        // Apply AGC to normalize signal amplitude
        let mut normalized = doppler_buffer.to_vec();
        self.agc.process_buffer(&mut normalized);

        // Filter doppler tone
        let mut filtered = normalized;
        self.bandpass.process_buffer(&mut filtered);

        // Get rotation period and frequency
        let samples_per_rotation = north_tick.period?;
        let rotation_freq = self.sample_rate / samples_per_rotation;
        let omega = 2.0 * PI * rotation_freq / self.sample_rate;

        // I/Q demodulation: correlate with cos and sin referenced to north tick
        // Reference time is the north tick (phase = 0 at north tick)
        let mut i_sum = 0.0;
        let mut q_sum = 0.0;
        let mut power_sum = 0.0;

        for (idx, &sample) in filtered.iter().enumerate() {
            let global_idx = self.sample_counter + idx;

            // Calculate phase relative to north tick
            let samples_from_tick = if global_idx >= north_tick.sample_index {
                (global_idx - north_tick.sample_index) as f32
            } else {
                // Skip buffers before the first tick
                self.sample_counter += doppler_buffer.len();
                return None;
            };

            let phase = omega * samples_from_tick;

            i_sum += sample * phase.cos();
            q_sum += sample * phase.sin();
            power_sum += sample * sample;
        }

        // Normalize by buffer length
        let n = filtered.len() as f32;
        let i = i_sum / n;
        let q = q_sum / n;

        // Calculate signal power for confidence metric
        let signal_power = power_sum / n;
        let correlation_magnitude = (i * i + q * q).sqrt();

        // Extract bearing directly from I/Q
        // Our signal is: A * sin(ω*t - φ) where φ is the bearing (note the minus!)
        // Correlating with sin(ω*t) and cos(ω*t) gives:
        // I ≈ A * sin(-φ) = -A * sin(φ)
        // Q ≈ A * cos(-φ) = A * cos(φ)
        // Therefore: -φ = atan2(I, Q), so φ = -atan2(I, Q)
        let bearing_phase = -i.atan2(q);

        // Normalize phase to [0, 2π)
        let mut normalized_phase = bearing_phase;
        while normalized_phase < 0.0 {
            normalized_phase += 2.0 * PI;
        }
        while normalized_phase >= 2.0 * PI {
            normalized_phase -= 2.0 * PI;
        }

        // Convert to bearing (0-360 degrees)
        let raw_bearing = phase_to_bearing(normalized_phase);

        // Apply smoothing
        let smoothed_bearing = self.bearing_smoother.add(raw_bearing);

        // Calculate confidence based on correlation magnitude and signal power
        let confidence = if signal_power > 0.01 {
            (correlation_magnitude / signal_power.sqrt()).min(1.0)
        } else {
            0.0
        };

        self.sample_counter += doppler_buffer.len();

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence,
            timestamp_samples: self.sample_counter,
        })
    }

    /// Reset calculator state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.sample_counter = 0;
        self.bearing_smoother.reset();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BearingMeasurement {
    pub bearing_degrees: f32,
    pub raw_bearing: f32,
    pub confidence: f32,
    #[allow(dead_code)]
    pub timestamp_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DopplerConfig;
    use crate::rdf::NorthTick;
    use std::f32::consts::PI;

    #[test]
    fn test_zero_crossing_bearing_calculator_creation() {
        use crate::config::AgcConfig;

        let doppler_config = DopplerConfig::default();
        let agc_config = AgcConfig::default();
        let sample_rate = 48000.0;
        let calc = ZeroCrossingBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1);
        assert!(calc.is_ok(), "Should be able to create ZeroCrossingBearingCalculator");
    }

    #[test]
    fn test_correlation_bearing_calculator_creation() {
        use crate::config::AgcConfig;
        use crate::rdf::CorrelationBearingCalculator;

        let doppler_config = DopplerConfig::default();
        let agc_config = AgcConfig::default();
        let sample_rate = 48000.0;
        let calc = CorrelationBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1);
        assert!(calc.is_ok(), "Should be able to create CorrelationBearingCalculator");
    }
}
