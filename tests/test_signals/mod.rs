pub mod generate;

pub use generate::generate_test_signal;
pub use generate::generate_test_signal_with_bearing_fn;

#[cfg(feature = "wav-export")]
pub use generate::save_wav;
