//! Wrap-safe statistics for circular quantities (bearing angles in degrees).
//!
//! Arithmetic statistics break at the 0/360 boundary: readings of 359 and 1
//! degrees average to 180 with an enormous spread. `CircularStats` computes
//! the circular mean from sine/cosine sums, dispersion from the resultant
//! length, and extremes as wrapped deviations about the mean.

/// Accumulates bearing angles in degrees and reports wrap-safe statistics.
#[derive(Debug, Default)]
pub struct CircularStats {
    angles_deg: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct CircularSummary {
    pub count: usize,
    /// Circular mean, in [0, 360).
    pub mean: f32,
    /// Circular standard deviation in degrees (sqrt(-2 ln R)).
    pub std_dev: f32,
    /// Mean plus the most negative wrapped deviation, in [0, 360).
    pub min: f32,
    /// Mean plus the most positive wrapped deviation, in [0, 360).
    pub max: f32,
    /// Spread between the extreme deviations, in degrees.
    pub range: f32,
}

impl CircularStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, degrees: f32) {
        self.angles_deg.push(degrees);
    }

    pub fn count(&self) -> usize {
        self.angles_deg.len()
    }

    pub fn summary(&self) -> Option<CircularSummary> {
        if self.angles_deg.is_empty() {
            return None;
        }
        let n = self.angles_deg.len() as f32;
        let (sum_cos, sum_sin) = self.angles_deg.iter().fold((0.0f32, 0.0f32), |(c, s), &a| {
            let r = a.to_radians();
            (c + r.cos(), s + r.sin())
        });
        let mean = sum_sin.atan2(sum_cos).to_degrees().rem_euclid(360.0);

        // Mean resultant length: 1 for identical angles, 0 for uniform spread.
        let resultant = (sum_cos * sum_cos + sum_sin * sum_sin).sqrt() / n;
        let std_dev = if resultant >= 1.0 {
            0.0
        } else if resultant <= f32::EPSILON {
            f32::INFINITY
        } else {
            (-2.0 * resultant.ln()).sqrt().to_degrees()
        };

        let wrap = |d: f32| (d + 180.0).rem_euclid(360.0) - 180.0;
        let (min_dev, max_dev) = self
            .angles_deg
            .iter()
            .map(|&a| wrap(a - mean))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), d| {
                (lo.min(d), hi.max(d))
            });

        Some(CircularSummary {
            count: self.angles_deg.len(),
            mean,
            std_dev,
            min: (mean + min_dev).rem_euclid(360.0),
            max: (mean + max_dev).rem_euclid(360.0),
            range: max_dev - min_dev,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_of(angles: &[f32]) -> CircularSummary {
        let mut s = CircularStats::new();
        for &a in angles {
            s.update(a);
        }
        s.summary().unwrap()
    }

    #[test]
    fn test_circular_stats_cluster_away_from_wrap() {
        let s = stats_of(&[89.0, 90.0, 91.0]);
        assert!((s.mean - 90.0).abs() < 0.01);
        assert!((s.min - 89.0).abs() < 0.01);
        assert!((s.max - 91.0).abs() < 0.01);
        assert!((s.range - 2.0).abs() < 0.01);
        assert!(s.std_dev < 1.0);
    }

    #[test]
    fn test_circular_stats_cluster_straddling_north() {
        // Arithmetic stats would report mean 180 and range 358 here.
        let s = stats_of(&[359.0, 1.0, 358.0, 2.0]);
        let mean_error = (s.mean - 0.0 + 180.0).rem_euclid(360.0) - 180.0;
        assert!(mean_error.abs() < 0.01, "mean {} not near 0", s.mean);
        assert!((s.range - 4.0).abs() < 0.01, "range {} not 4", s.range);
        assert!((s.min - 358.0).abs() < 0.01, "min {}", s.min);
        assert!((s.max - 2.0).abs() < 0.01, "max {}", s.max);
        assert!(s.std_dev < 3.0, "std_dev {}", s.std_dev);
    }

    #[test]
    fn test_circular_stats_empty_and_single() {
        assert!(CircularStats::new().summary().is_none());
        let s = stats_of(&[45.0]);
        assert_eq!(s.count, 1);
        assert!((s.mean - 45.0).abs() < 0.01);
        assert_eq!(s.range, 0.0);
        // f32 rounding leaves the resultant marginally below 1, so allow a
        // hair of numerical noise instead of exactly 0.
        assert!(s.std_dev < 0.1, "std_dev {}", s.std_dev);
    }
}
