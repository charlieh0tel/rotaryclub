pub mod generate;
pub mod noise;

pub use generate::generate_test_signal;
pub use generate::generate_test_signal_with_bearing_fn;
pub use noise::{NoiseConfig, apply_noise, generate_noisy_test_signal};

#[cfg(feature = "wav-export")]
pub use generate::save_wav;
