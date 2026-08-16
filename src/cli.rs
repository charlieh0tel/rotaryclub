//! Argument parsing shared by the CLI and GUI binaries.
//!
//! These live in the library so the two binaries cannot drift: the GUI once
//! carried its own bare `f32` for `--north-offset`, which rejected negative
//! offsets at the parser and accepted NaN — the exact pair of failures the
//! CLI's parser exists to prevent.

use std::path::{Path, PathBuf};

/// Reject a non-finite north offset.
///
/// It reaches the bearing itself rather than a quality figure, so a NaN here
/// is not a confidence that reads badly -- it is a bearing that is NaN. The
/// JSON formatter renders that as null and the KN5R formatter refuses the
/// sentence, but the honest place to stop it is before it enters at all.
pub fn parse_north_offset(s: &str) -> Result<f32, String> {
    let offset: f32 = s.parse().map_err(|_| format!("invalid number: {s}"))?;
    if offset.is_finite() {
        Ok(offset)
    } else {
        Err("north offset must be a finite number of degrees".to_string())
    }
}

/// Reject an output rate the pipeline cannot honor.
///
/// The lower bound is not pedantry: the rate becomes a Duration, and
/// Duration::from_secs_f32 panics rather than saturating once the interval
/// exceeds what it can hold. One output per hour is already absurd.
pub fn parse_output_rate(s: &str) -> Result<f32, String> {
    let rate: f32 = s.parse().map_err(|_| format!("invalid number: {s}"))?;
    if rate.is_finite() && rate >= 1.0 / 3600.0 {
        Ok(rate)
    } else {
        Err("output rate must be a finite number of Hz, at least 1/3600".to_string())
    }
}

/// Whether two paths name the same file, by device and inode where the
/// filesystem can say, falling back to canonical-path equality.
///
/// Used to refuse `--dump-audio` pointing at the input: the dump writer
/// truncates its path on creation, so that spelling destroys the recording
/// being read. The dump path usually does not exist yet, so on that side it
/// is the parent directory plus file name that gets canonicalised.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    let canonical = |p: &Path| -> Option<PathBuf> {
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        let parent = p.parent()?;
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        Some(parent.canonicalize().ok()?.join(p.file_name()?))
    };
    match (canonical(a), canonical(b)) {
        (Some(ca), Some(cb)) => ca == cb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn north_offset_accepts_negative_rejects_non_finite() {
        assert_eq!(parse_north_offset("-5.5"), Ok(-5.5));
        assert!(parse_north_offset("NaN").is_err());
        assert!(parse_north_offset("inf").is_err());
    }

    #[test]
    fn output_rate_bounds() {
        assert!(parse_output_rate("10").is_ok());
        assert!(parse_output_rate("1e-30").is_err());
        assert!(parse_output_rate("NaN").is_err());
    }

    #[test]
    fn same_file_sees_through_relative_spellings() {
        let dir = std::env::temp_dir();
        let a = dir.join("rc_same_file_test.tmp");
        std::fs::write(&a, b"x").unwrap();
        assert!(same_file(&a, &a));
        let missing = dir.join("rc_same_file_missing.tmp");
        assert!(!same_file(&a, &missing));
        std::fs::remove_file(&a).ok();
    }
}
