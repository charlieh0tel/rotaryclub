mod measure;
mod noise;
mod signal;

pub use measure::{
    BearingMeasurement, ErrorStats, angle_error, circular_mean_degrees, measure_bearing,
    measure_error_across_bearings,
};
pub use noise::{
    AdditiveNoiseConfig, DoublingConfig, FadingConfig, FadingType, FrequencyDriftConfig,
    ImpulseNoiseConfig, MultipathComponent, MultipathConfig, NoiseConfig, apply_noise,
    generate_noisy_test_signal, signal_power,
};
pub use signal::{
    NORTH_TICK_AMPLITUDE, NORTH_TICK_PULSE_WIDTH_RADIANS, generate_doppler_signal_for_bearing,
    generate_test_signal, generate_test_signal_with_bearing_fn,
};
