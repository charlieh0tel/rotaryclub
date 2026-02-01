pub mod agc;
pub mod detector;
pub mod filters;
pub mod math;

pub use agc::AutomaticGainControl;
pub use detector::{PeakDetector, ZeroCrossingDetector};
pub use filters::{BandpassFilter, HighpassFilter};
pub use math::{MovingAverage, phase_to_bearing};
