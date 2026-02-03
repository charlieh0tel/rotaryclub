pub mod agc;
pub mod filter;
pub mod fir_bandpass;
pub mod iir_butterworth_bandpass;
pub mod iir_butterworth_highpass;
pub mod moving_average;
pub mod peak_detector;
pub mod zero_crossing_detector;

pub use agc::AutomaticGainControl;
#[allow(unused_imports)]
pub use filter::Filter;
pub use fir_bandpass::FirBandpass;
#[allow(unused_imports)]
pub use iir_butterworth_bandpass::IirButterworthBandpass;
pub use iir_butterworth_highpass::IirButterworthHighpass;
pub use moving_average::MovingAverage;
pub use peak_detector::PeakDetector;
pub use zero_crossing_detector::ZeroCrossingDetector;
