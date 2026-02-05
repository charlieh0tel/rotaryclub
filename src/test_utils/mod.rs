mod generate;
mod measure;
mod noise;

pub use generate::{
    NORTH_TICK_AMPLITUDE, NORTH_TICK_PULSE_WIDTH_RADIANS, generate_test_signal,
    generate_test_signal_with_bearing_fn,
};
pub use measure::{
    BearingMeasurement, ErrorStats, angle_error, measure_bearing, measure_error_across_bearings,
};
pub use noise::{
    AdditiveNoiseConfig, DoublingConfig, FadingConfig, FadingType, FrequencyDriftConfig,
    ImpulseNoiseConfig, MultipathComponent, MultipathConfig, NoiseConfig, apply_noise,
    generate_doppler_signal_for_bearing, generate_noisy_test_signal, signal_power,
};

#[cfg(feature = "wav-export")]
pub use generate::save_wav;
