pub mod agc;
pub mod detector;
pub mod filters;
pub mod math;
pub mod moving_average;

pub use agc::AutomaticGainControl;
pub use detector::{PeakDetector, ZeroCrossingDetector};
pub use filters::{BandpassFilter, HighpassFilter};
pub use math::phase_to_bearing;
pub use moving_average::MovingAverage;
