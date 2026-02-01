# Contributing to Rotary Club

Welcome to the Rotary Club project! We're excited that you're interested in contributing to this pseudo-Doppler radio direction finding system. Whether you're a seasoned ham radio operator, a software developer, or both, we welcome your contributions.

Like the amateur radio community itself, this project thrives on collaboration, experimentation, and shared knowledge. We aim to build a high-quality, reliable RDF system that serves the ham radio community well.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How to Report Bugs](#how-to-report-bugs)
- [How to Suggest Enhancements](#how-to-suggest-enhancements)
- [Development Setup](#development-setup)
- [Code Style Requirements](#code-style-requirements)
- [Testing Requirements](#testing-requirements)
- [Commit Message Guidelines](#commit-message-guidelines)
- [Pull Request Process](#pull-request-process)
- [Documentation Requirements](#documentation-requirements)

## Code of Conduct

### Our Pledge

In the spirit of amateur radio's tradition of mutual assistance and friendly collaboration, we pledge to make participation in this project a welcoming and respectful experience for everyone, regardless of:

- Experience level (Technician to Extra, beginner programmer to expert)
- Background, nationality, or native language
- Age, gender identity, or personal characteristics

### Our Standards

- Be respectful and constructive in discussions
- Welcome newcomers and help them get started
- Share knowledge freely, as hams have done for over a century
- Accept constructive criticism gracefully
- Focus on what's best for the project and the community
- Recognize that we all make mistakes and learn from them

### Unacceptable Behavior

- Harassment, insults, or discriminatory language
- Trolling, inflammatory comments, or personal attacks
- Publishing others' private information without permission
- Any conduct that would be inappropriate in a professional or amateur radio setting

If you experience or witness unacceptable behavior, please report it to the project maintainers.

## How to Report Bugs

Before submitting a bug report:

1. **Check existing issues** to see if it's already been reported
2. **Update to the latest version** to see if the issue persists
3. **Gather information** about your setup and the problem

### Bug Report Template

When filing a bug report, please include:

**Environment:**
- Rotary Club version (e.g., `rotaryclub --version` or git commit)
- Operating system and version (e.g., Ubuntu 24.04, Raspberry Pi OS)
- Rust version (e.g., `rustc --version`)
- Audio hardware (e.g., "USB sound card", "built-in audio")

**Description:**
- What you expected to happen
- What actually happened
- Steps to reproduce the issue

**RDF Hardware Details (if applicable):**
- Antenna array configuration
- Rotation frequency
- Radio model and frequency
- Signal characteristics

**Logs:**
Run with verbose logging (`-v` or `-vv`) and include relevant output:
```bash
rotaryclub -vv 2>&1 | tee debug.log
```

**Additional Context:**
- Screenshots, audio recordings, or WAV files that demonstrate the issue
- Any error messages or stack traces

## How to Suggest Enhancements

We welcome suggestions for new features and improvements! Before submitting:

1. **Check existing issues** to see if it's already been proposed
2. **Consider the scope** - does it fit the project's goals?
3. **Think about implementation** - is it technically feasible?

### Enhancement Proposal Template

**Use Case:**
Describe the problem or scenario this enhancement addresses. Include amateur radio context if relevant (e.g., "During fox hunts, operators need...")

**Proposed Solution:**
Describe your suggested implementation. Be as specific as possible.

**Alternatives Considered:**
What other approaches could solve this problem?

**Technical Considerations:**
- Performance impact
- Compatibility with existing features
- Hardware requirements
- Signal processing implications

**Additional Context:**
- References to papers, standards, or other RDF systems
- Examples from real-world usage
- Links to relevant resources

## Development Setup

### Prerequisites

**Required:**
- Rust 1.70 or later ([rustup.rs](https://rustup.rs/))
- Linux with ALSA support
- libasound2-dev (development headers)

**Optional:**
- `cargo-deb` for building Debian packages
- An actual RDF antenna setup for testing (but synthetic signals work too!)

### Installation

```bash
# Install system dependencies (Debian/Ubuntu)
sudo apt-get install libasound2-dev

# Clone the repository
git clone https://github.com/yourusername/rotaryclub.git
cd rotaryclub

# Build the project
cargo build

# Run tests to verify setup
cargo test

# Try the examples
cargo run --example synthetic_rdf
```

### Project Structure

```
rotaryclub/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── config.rs            # Configuration and constants
│   ├── audio/               # Audio I/O and processing
│   ├── bearing/             # Bearing calculation algorithms
│   ├── dsp/                 # Digital signal processing
│   └── north/               # North reference tracking
├── examples/                # Example programs
├── tests/                   # Integration tests
├── data/                    # Sample audio files
├── DESIGN.md                # System architecture documentation
└── README.md                # User documentation
```

### Development Workflow

1. Create a feature branch: `git checkout -b feature/your-feature-name`
2. Make your changes
3. Run tests: `cargo test`
4. Run formatting: `cargo fmt`
5. Run linter: `cargo clippy -- -D warnings`
6. Commit your changes (see commit guidelines below)
7. Push and create a pull request

## Code Style Requirements

We follow standard Rust conventions with strict enforcement.

### Formatting

**All code must be formatted with `rustfmt`:**

```bash
# Format all code
cargo fmt

# Check formatting without modifying files
cargo fmt -- --check
```

Configuration is in `rustfmt.toml` (or uses default settings). Do not commit unformatted code.

### Linting

**All code must pass `clippy` with warnings treated as errors:**

```bash
# Run clippy with strict settings
cargo clippy -- -D warnings

# Also check tests and examples
cargo clippy --all-targets -- -D warnings
```

Fix all warnings before submitting. If you believe a warning is a false positive, add a targeted `#[allow(...)]` attribute with a comment explaining why.

### Code Quality Guidelines

- **Write clear, self-documenting code** with descriptive variable names
- **Add comments for complex algorithms**, especially signal processing math
- **Use type-safe abstractions** rather than raw primitives where appropriate
- **Handle errors properly** - avoid unwrap() in production code
- **Keep functions focused** - each function should do one thing well
- **Minimize unsafe code** - if needed, document safety invariants thoroughly

### Documentation Comments

Use Rust doc comments (`///`) for public APIs:

```rust
/// Calculates bearing angle from Doppler-shifted audio using I/Q correlation.
///
/// This method performs better in noisy conditions compared to zero-crossing
/// detection by correlating the Doppler tone with quadrature reference signals.
///
/// # Arguments
///
/// * `doppler_samples` - Audio samples containing the Doppler tone
/// * `rotation_freq` - Antenna rotation frequency in Hz
/// * `sample_rate` - Audio sample rate in Hz
///
/// # Returns
///
/// Bearing angle in degrees (0-360°), where 0° is north.
///
/// # Example
///
/// ```
/// let bearing = calculate_bearing_correlation(&samples, 30.0, 48000.0);
/// println!("Target bearing: {:.1}°", bearing);
/// ```
pub fn calculate_bearing_correlation(
    doppler_samples: &[f32],
    rotation_freq: f32,
    sample_rate: f32,
) -> f32 {
    // Implementation...
}
```

## Testing Requirements

**All contributions must include tests and all tests must pass.**

### Running Tests

```bash
# Run all tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run specific test
cargo test test_bearing_calculation

# Run with all features
cargo test --all-features
```

### Test Requirements

1. **Unit tests** for individual functions (in same file or `tests/` module)
2. **Integration tests** for end-to-end functionality (in `tests/` directory)
3. **Example programs** should run without errors
4. **Documentation examples** should compile and run (tested by `cargo test`)

### Writing Tests

**Unit test example:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn test_bearing_calculation_known_signal() {
        // Generate synthetic signal at known bearing
        let samples = generate_test_signal(90.0, 30.0, 48000.0);

        let bearing = calculate_bearing_correlation(&samples, 30.0, 48000.0);

        // Allow small tolerance due to numerical precision
        assert_relative_eq!(bearing, 90.0, epsilon = 2.0);
    }

    #[test]
    fn test_north_wraparound() {
        // Test that 359° and 1° are close
        assert!(angle_difference(359.0, 1.0) < 5.0);
    }
}
```

**Integration test example** (`tests/bearing_integration.rs`):

```rust
use rotaryclub::bearing::BearingCalculator;

#[test]
fn test_full_pipeline_with_wav_file() {
    let audio_data = load_test_wav("data/doppler-test-2023-04-10-ft-70d.wav");
    let calculator = BearingCalculator::new(48000.0);

    let bearing = calculator.process(&audio_data);

    assert!(bearing >= 0.0 && bearing < 360.0);
}
```

### Test Coverage

While we don't enforce a specific coverage percentage, aim to test:

- Normal operation paths
- Edge cases (e.g., 0°/360° wraparound)
- Error conditions
- Signal processing correctness with known inputs
- Different configuration options

## Commit Message Guidelines

Write clear, descriptive commit messages that explain **why** a change was made.

### Format

```
Short summary (50 chars or less)

More detailed explanation if needed. Wrap at 72 characters.
Explain the problem this commit solves and why this approach
was chosen.

- Bullet points are fine
- Reference issue numbers: Fixes #123, See #456

Technical details about signal processing changes, performance
implications, or compatibility notes.
```

### Examples

**Good:**
```
Fix phase calculation error causing 180° bearing offset

The I/Q correlation was using atan instead of atan2, which doesn't
handle all quadrants correctly. This caused bearings in the western
hemisphere (180-360°) to wrap incorrectly.

Fixes #42
```

**Good:**
```
Add DPLL-based north tracking for improved accuracy

Replaces simple threshold detection with a digital phase-locked loop
that maintains phase lock even with noisy north reference pulses.
Improves bearing stability by ~0.5° RMS in field testing.

See DESIGN.md for algorithm details.
```

**Too vague:**
```
Fix bug
Update README
Changes
```

### Commit Practices

- **Make atomic commits** - each commit should be a single logical change
- **Commit working code** - every commit should build and pass tests
- **Write in imperative mood** - "Add feature" not "Added feature"
- **Reference issues** when relevant

## Pull Request Process

### Before Submitting

1. **Update your branch** with the latest main:
   ```bash
   git checkout main
   git pull
   git checkout your-feature-branch
   git rebase main
   ```

2. **Run the full test suite:**
   ```bash
   cargo test
   cargo clippy -- -D warnings
   cargo fmt -- --check
   ```

3. **Update documentation** if you changed functionality

4. **Add examples** if you added new features

### PR Description Template

```markdown
## Summary

Brief description of what this PR does.

## Motivation

Why is this change needed? What problem does it solve?

## Changes

- List of specific changes made
- Technical approach taken
- Any tradeoffs or design decisions

## Testing

How was this tested?
- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] Tested with actual RDF hardware (if applicable)
- [ ] Tested with synthetic signals
- [ ] Examples run successfully

## Documentation

- [ ] Code comments added for complex logic
- [ ] Public API documented with doc comments
- [ ] DESIGN.md updated (if architecture changed)
- [ ] README.md updated (if user-facing changes)

## Checklist

- [ ] Code follows project style guidelines
- [ ] All tests pass
- [ ] No clippy warnings
- [ ] Code is formatted with rustfmt
- [ ] Commit messages follow guidelines
- [ ] No unnecessary dependencies added

## Additional Context

References, screenshots, benchmark results, field test notes, etc.
```

### Review Process

1. **Automated checks** must pass (formatting, clippy, tests)
2. **Maintainer review** - be patient, we're volunteers like you
3. **Address feedback** - discussion and iteration are normal
4. **Final approval** - maintainer will merge when ready

### After Merge

- Your contribution will be included in the next release
- You'll be credited in release notes
- Consider sticking around to help with related issues

## Documentation Requirements

### Code Documentation

- **Public APIs** must have doc comments (`///`)
- **Complex algorithms** should have inline comments explaining the math
- **Signal processing** should reference theory or papers when applicable
- **Configuration constants** should explain their purpose and typical values

### User Documentation

Update relevant documentation when you make changes:

- **README.md** - user-facing features, installation, quick start
- **DESIGN.md** - system architecture, algorithms, signal processing theory
- **Examples** - add example programs for new features
- **CLI help** - keep `--help` output accurate

### Amateur Radio Context

When documenting RDF-specific features:

- Explain amateur radio terminology for non-hams
- Reference standard practices in the RDF community
- Include typical use cases (fox hunting, direction finding, etc.)
- Provide frequency ranges and power levels as examples

## Questions?

If you have questions about contributing:

- Check existing [issues](https://github.com/yourusername/rotaryclub/issues)
- Open a new issue with the "question" label
- Reach out to maintainers

We're here to help! Remember: every expert was once a beginner, and every contribution - no matter how small - helps improve this project for the entire ham radio community.

**73 de Rotary Club team**
