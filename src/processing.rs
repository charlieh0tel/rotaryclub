use crate::audio::AudioRingBuffer;
use crate::config::{AudioConfig, BearingMethod, RdfConfig};
use crate::error::Result;
use crate::rdf::{
    BearingCalculator, BearingMeasurement, CorrelationBearingCalculator, NorthReferenceTracker,
    NorthTick, NorthTracker, ZeroCrossingBearingCalculator,
};
use crate::signal_processing::DcRemover;

pub struct TickResult {
    pub north_tick: NorthTick,
    pub bearing: Option<BearingMeasurement>,
}

pub struct RdfProcessor {
    north_tracker: NorthReferenceTracker,
    bearing_calc: Option<Box<dyn BearingCalculator>>,
    dc_remover_doppler: DcRemover,
    dc_remover_north: DcRemover,
    ring_buffer: AudioRingBuffer,
    audio_config: AudioConfig,
    last_north_tick: Option<NorthTick>,
    remove_dc: bool,
    doppler_buf: Vec<f32>,
}

impl RdfProcessor {
    pub fn new(config: &RdfConfig, remove_dc: bool, compute_bearings: bool) -> Result<Self> {
        let sample_rate = config.audio.sample_rate as f32;
        let north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;

        let bearing_calc: Option<Box<dyn BearingCalculator>> = if compute_bearings {
            Some(match config.doppler.method {
                BearingMethod::ZeroCrossing => Box::new(ZeroCrossingBearingCalculator::new(
                    &config.doppler,
                    &config.agc,
                    config.bearing.confidence_weights,
                    sample_rate,
                    config.bearing.smoothing_window,
                )?),
                BearingMethod::Correlation => Box::new(CorrelationBearingCalculator::new(
                    &config.doppler,
                    &config.agc,
                    config.bearing.confidence_weights,
                    sample_rate,
                    config.bearing.smoothing_window,
                )?),
            })
        } else {
            None
        };

        Ok(Self {
            north_tracker,
            bearing_calc,
            dc_remover_doppler: DcRemover::with_cutoff(sample_rate, 1.0),
            dc_remover_north: DcRemover::with_cutoff(sample_rate, 1.0),
            ring_buffer: AudioRingBuffer::new(),
            audio_config: config.audio.clone(),
            last_north_tick: None,
            remove_dc,
            doppler_buf: Vec::new(),
        })
    }

    pub fn process_audio(&mut self, interleaved: &[f32]) -> Vec<TickResult> {
        self.ring_buffer.push_interleaved(interleaved);

        let samples = self.ring_buffer.latest(interleaved.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (mut doppler, mut north) = self.audio_config.split_channels(&stereo_pairs);

        if self.remove_dc {
            self.dc_remover_doppler.process(&mut doppler);
            self.dc_remover_north.process(&mut north);
        }

        let north_ticks = self.north_tracker.process_buffer(&north);

        if let Some(tick) = north_ticks.last() {
            self.last_north_tick = Some(*tick);
        }

        if let Some(ref mut calc) = self.bearing_calc {
            calc.preprocess(&doppler);
        }

        self.doppler_buf = doppler;

        let results = north_ticks
            .iter()
            .map(|tick| {
                let bearing = self
                    .bearing_calc
                    .as_mut()
                    .and_then(|calc| calc.process_tick(tick));
                TickResult {
                    north_tick: *tick,
                    bearing,
                }
            })
            .collect();

        if let Some(ref mut calc) = self.bearing_calc {
            calc.advance_buffer();
        }

        results
    }

    pub fn process_signal(&mut self, interleaved: &[f32]) -> Vec<TickResult> {
        let chunk_size = self.audio_config.buffer_size * 2;
        let mut all_results = Vec::new();
        for chunk in interleaved.chunks(chunk_size) {
            all_results.extend(self.process_audio(chunk));
        }
        all_results
    }

    pub fn last_north_tick(&self) -> Option<&NorthTick> {
        self.last_north_tick.as_ref()
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        self.north_tracker.rotation_frequency()
    }

    pub fn phase_error_variance(&self) -> Option<f32> {
        self.north_tracker.phase_error_variance()
    }

    pub fn filtered_doppler(&self) -> &[f32] {
        self.bearing_calc
            .as_ref()
            .map(|c| c.filtered_buffer())
            .unwrap_or(&self.doppler_buf)
    }

    pub fn filtered_north(&self) -> &[f32] {
        self.north_tracker.filtered_buffer()
    }
}

#[cfg(all(test, feature = "simulation"))]
mod tests {
    use super::*;
    use crate::config::{BearingMethod, RdfConfig};
    use crate::simulation::{angle_error, circular_mean_degrees, generate_test_signal};

    fn default_config() -> RdfConfig {
        RdfConfig::default()
    }

    fn extract_bearings(results: &[TickResult]) -> Vec<f32> {
        results
            .iter()
            .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
            .collect()
    }

    fn mean_bearing_skipping_warmup(results: &[TickResult], skip: usize) -> Option<f32> {
        let bearings = extract_bearings(results);
        if bearings.len() > skip {
            circular_mean_degrees(&bearings[skip..])
        } else {
            circular_mean_degrees(&bearings)
        }
    }

    #[test]
    fn test_process_signal_bearing_accuracy() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;

        for expected_bearing in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
            let signal = generate_test_signal(0.5, sample_rate, rotation_hz, expected_bearing);
            let mut processor = RdfProcessor::new(&config, false, true).unwrap();
            let results = processor.process_signal(&signal);

            let measured = mean_bearing_skipping_warmup(&results, 3)
                .unwrap_or_else(|| panic!("No bearings for {}°", expected_bearing));

            let error = angle_error(measured, expected_bearing).abs();
            assert!(
                error < 3.0,
                "Bearing {}°: measured {:.1}°, error {:.1}° exceeds 3° threshold",
                expected_bearing,
                measured,
                error
            );
        }
    }

    #[test]
    fn test_process_signal_both_methods() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;
        let expected_bearing = 135.0;

        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, expected_bearing);

        let mut zc_config = config.clone();
        zc_config.doppler.method = BearingMethod::ZeroCrossing;
        let mut corr_config = config;
        corr_config.doppler.method = BearingMethod::Correlation;

        let mut zc_proc = RdfProcessor::new(&zc_config, false, true).unwrap();
        let mut corr_proc = RdfProcessor::new(&corr_config, false, true).unwrap();

        let zc_results = zc_proc.process_signal(&signal);
        let corr_results = corr_proc.process_signal(&signal);

        let zc_bearing = mean_bearing_skipping_warmup(&zc_results, 3).expect("No ZC bearings");
        let corr_bearing =
            mean_bearing_skipping_warmup(&corr_results, 3).expect("No Correlation bearings");

        let zc_error = angle_error(zc_bearing, expected_bearing).abs();
        let corr_error = angle_error(corr_bearing, expected_bearing).abs();
        let method_diff = angle_error(zc_bearing, corr_bearing).abs();

        assert!(
            zc_error < 3.0,
            "ZC error {:.1}° exceeds 3° threshold",
            zc_error
        );
        assert!(
            corr_error < 3.0,
            "Correlation error {:.1}° exceeds 3° threshold",
            corr_error
        );
        assert!(
            method_diff < 2.0,
            "Methods disagree by {:.1}° (exceeds 2° threshold)",
            method_diff
        );
    }

    #[test]
    fn test_north_tick_detection() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;

        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, 0.0);
        let mut processor = RdfProcessor::new(&config, false, true).unwrap();
        let results = processor.process_signal(&signal);

        let expected_ticks = (rotation_hz * 0.5) as usize;
        let margin = (expected_ticks as f32 * 0.15) as usize;
        assert!(
            results.len() >= expected_ticks - margin && results.len() <= expected_ticks + margin,
            "Expected ~{} ticks, got {}",
            expected_ticks,
            results.len()
        );

        assert!(
            processor.last_north_tick().is_some(),
            "last_north_tick() should be Some after processing"
        );

        let freq = processor.rotation_frequency();
        assert!(freq.is_some(), "rotation_frequency() should be Some");
        let freq = freq.unwrap();
        assert!(
            (freq - rotation_hz).abs() < 50.0,
            "Rotation frequency {:.1} Hz too far from expected {:.1} Hz",
            freq,
            rotation_hz
        );
    }

    #[test]
    fn test_compute_bearings_false() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;

        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, 90.0);
        let mut processor = RdfProcessor::new(&config, false, false).unwrap();
        let results = processor.process_signal(&signal);

        assert!(!results.is_empty(), "Should still detect north ticks");
        assert!(
            results.iter().all(|r| r.bearing.is_none()),
            "All bearings should be None when compute_bearings is false"
        );
        assert!(
            processor.last_north_tick().is_some(),
            "North ticks should still be tracked"
        );
    }

    #[test]
    fn test_process_audio_chunked_matches_process_signal() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;

        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, 200.0);

        let mut proc_whole = RdfProcessor::new(&config, false, true).unwrap();
        let whole_results = proc_whole.process_signal(&signal);

        let mut proc_chunked = RdfProcessor::new(&config, false, true).unwrap();
        let chunk_size = config.audio.buffer_size * 2;
        let mut chunked_results = Vec::new();
        for chunk in signal.chunks(chunk_size) {
            chunked_results.extend(proc_chunked.process_audio(chunk));
        }

        let whole_bearings = extract_bearings(&whole_results);
        let chunked_bearings = extract_bearings(&chunked_results);

        assert_eq!(
            whole_bearings.len(),
            chunked_bearings.len(),
            "Different number of bearings: process_signal={}, chunked={}",
            whole_bearings.len(),
            chunked_bearings.len()
        );

        for (i, (w, c)) in whole_bearings
            .iter()
            .zip(chunked_bearings.iter())
            .enumerate()
        {
            assert!(
                (w - c).abs() < 1e-6,
                "Bearing {} differs: process_signal={}, chunked={}",
                i,
                w,
                c
            );
        }
    }

    #[test]
    fn test_dc_removal() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;
        let expected_bearing = 90.0;

        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, expected_bearing);
        let dc_offset = 0.5;
        let signal_with_dc: Vec<f32> = signal.iter().map(|s| s + dc_offset).collect();

        let mut proc_no_dc = RdfProcessor::new(&config, false, true).unwrap();
        let mut proc_dc = RdfProcessor::new(&config, true, true).unwrap();

        let results_no_dc = proc_no_dc.process_signal(&signal_with_dc);
        let results_dc = proc_dc.process_signal(&signal_with_dc);

        let bearing_no_dc = mean_bearing_skipping_warmup(&results_no_dc, 3);
        let bearing_dc = mean_bearing_skipping_warmup(&results_dc, 3);

        if let Some(b) = bearing_dc {
            let error = angle_error(b, expected_bearing).abs();
            assert!(
                error < 10.0,
                "DC removal bearing error {:.1}° exceeds 10° threshold",
                error
            );
        }

        if let (Some(b_dc), Some(b_no_dc)) = (bearing_dc, bearing_no_dc) {
            let error_dc = angle_error(b_dc, expected_bearing).abs();
            let error_no_dc = angle_error(b_no_dc, expected_bearing).abs();
            assert!(
                error_dc <= error_no_dc + 5.0,
                "DC removal made accuracy worse: {:.1}° vs {:.1}° without",
                error_dc,
                error_no_dc
            );
        }
    }

    #[test]
    fn test_filtered_buffers_nonempty() {
        let config = default_config();
        let rotation_hz = config.doppler.expected_freq;
        let sample_rate = config.audio.sample_rate;

        let signal = generate_test_signal(0.1, sample_rate, rotation_hz, 0.0);
        let mut processor = RdfProcessor::new(&config, false, true).unwrap();
        processor.process_signal(&signal);

        assert!(
            !processor.filtered_doppler().is_empty(),
            "filtered_doppler() should be non-empty after processing"
        );
        assert!(
            !processor.filtered_north().is_empty(),
            "filtered_north() should be non-empty after processing"
        );
    }

    #[test]
    fn test_empty_signal() {
        let config = default_config();
        let mut processor = RdfProcessor::new(&config, false, true).unwrap();
        let results = processor.process_signal(&[]);

        assert!(results.is_empty(), "Empty input should produce no results");
        assert!(
            processor.last_north_tick().is_none(),
            "No north tick expected for empty input"
        );
    }

    #[test]
    fn test_smoothing_window_zero_rejected() {
        let mut config = default_config();
        config.bearing.smoothing_window = 0;

        match RdfProcessor::new(&config, false, true) {
            Err(err) => match err {
                crate::error::RdfError::Config(msg) => {
                    assert!(
                        msg.contains("smoothing_window"),
                        "Unexpected config error message: {}",
                        msg
                    );
                }
                _ => panic!("Expected configuration error for zero smoothing window"),
            },
            Ok(_) => panic!("Expected zero smoothing window to be rejected"),
        }
    }
}
