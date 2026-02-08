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
                    sample_rate,
                    config.bearing.smoothing_window,
                )?),
                BearingMethod::Correlation => Box::new(CorrelationBearingCalculator::new(
                    &config.doppler,
                    &config.agc,
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
