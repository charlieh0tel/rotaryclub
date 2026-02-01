# Release Process

This document describes how to create a new release of Rotary Club.

## Automated Release Workflow

Releases are automated via GitHub Actions. When you push a version tag, the workflow automatically:

1. Builds the release binary with full optimizations
2. Runs all tests to verify correctness
3. Creates a Debian package (.deb)
4. Creates a GitHub release for the tag
5. Attaches the .deb package to the release
6. Generates release notes from commits

## Creating a Release

### 1. Update Version Number

Edit `Cargo.toml` and update the version:

```toml
[package]
name = "rotaryclub"
version = "0.2.0"  # Update this
edition = "2024"
```

### 2. Update CHANGELOG (if you have one)

Document what's new in this release.

### 3. Commit Version Changes

```bash
git add Cargo.toml
git commit -m "Bump version to 0.2.0"
```

### 4. Create and Push Version Tag

Create an annotated tag with the version number prefixed with `v`:

```bash
# Create annotated tag
git tag -a v0.2.0 -m "Release version 0.2.0"

# Push the commit
git push origin main

# Push the tag
git push origin v0.2.0
```

**Important:** The tag must start with `v` (e.g., `v0.1.0`, `v1.0.0`) to trigger the release workflow.

### 5. Monitor the Build

1. Go to your repository on GitHub
2. Click the "Actions" tab
3. You should see the "Build Debian Package" workflow running
4. Wait for it to complete (usually 5-10 minutes)

### 6. Verify the Release

1. Go to the "Releases" page on GitHub
2. The new release should appear with:
   - The tag name (e.g., v0.2.0)
   - Auto-generated release notes
   - The Debian package attached as an asset

## Version Tag Format

- **Format**: `vMAJOR.MINOR.PATCH`
- **Examples**: `v0.1.0`, `v1.0.0`, `v1.2.3`
- **Must start with lowercase `v`**

The workflow is triggered by tags matching the pattern `v*`.

## Workflow Details

The release workflow (`.github/workflows/build-deb.yml`) performs these steps:

```yaml
Trigger: Push tag matching v*
  ↓
Install Rust + dependencies
  ↓
Build release binary (optimized)
  ↓
Run all tests
  ↓
Build Debian package with cargo-deb
  ↓
Create GitHub release
  ↓
Upload .deb as release asset
```

## Manual Testing Before Release

Before creating a release tag, test locally:

```bash
# Run all tests
cargo test --verbose

# Check for warnings
cargo clippy --all-targets -- -D warnings

# Check formatting
cargo fmt -- --check

# Build release binary
cargo build --release

# Build Debian package
cargo deb

# Test the .deb package
sudo dpkg -i target/debian/rotaryclub_*.deb
rotaryclub --help
```

## Troubleshooting

### Tag Already Exists

If you need to recreate a tag:

```bash
# Delete local tag
git tag -d v0.2.0

# Delete remote tag
git push origin :refs/tags/v0.2.0

# Recreate and push
git tag -a v0.2.0 -m "Release version 0.2.0"
git push origin v0.2.0
```

### Workflow Failed

1. Check the Actions tab for error details
2. Common issues:
   - Tests failed: Fix the failing tests and create a new tag
   - Build failed: Check dependencies in workflow
   - Clippy warnings: Fix code issues
3. Delete the failed release from GitHub Releases page
4. Fix the issue and create a new patch version tag

## Pre-release Versions

For pre-release testing, use tags like:

```bash
git tag -a v0.2.0-rc1 -m "Release candidate 1"
git push origin v0.2.0-rc1
```

These will trigger the workflow and create a release marked as "pre-release" if you manually edit the release on GitHub.

## Artifact Retention

- **Release artifacts**: Permanent (attached to GitHub release)
- **CI artifacts**: 30 days retention
- **Build caches**: Managed automatically by GitHub Actions

## Download URLs

After release, the .deb package will be available at:

```
https://github.com/yourusername/rotaryclub/releases/download/v0.2.0/rotaryclub_0.2.0-1_amd64.deb
```

Users can download and install with:

```bash
wget https://github.com/yourusername/rotaryclub/releases/download/v0.2.0/rotaryclub_0.2.0-1_amd64.deb
sudo dpkg -i rotaryclub_0.2.0-1_amd64.deb
```
