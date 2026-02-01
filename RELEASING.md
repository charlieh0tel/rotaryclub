# Release Process

This document describes how to create a new release of Rotary Club.

## Automated Release Workflow

Releases are automated via GitHub Actions. When you push a version
tag, the workflow automatically:

1. Builds release binaries for amd64 and arm64
2. Runs tests to verify correctness
3. Creates Debian packages (.deb) for both architectures
4. Creates a GitHub release for the tag
5. Attaches the .deb packages to the release
6. Generates release notes from commits

## Creating a Release

### 1. Update Version Number

Edit `Cargo.toml` and update the version:

```toml
[package]
name = "rotaryclub"
version = "1.3.0"  # Update this
```

### 2. Update Cargo.lock

```bash
cargo build --release
```

### 3. Commit Version Changes

```bash
git add Cargo.toml Cargo.lock
git commit -m "Bump version to 1.3.0"
```

### 4. Push to GitHub

```bash
git push origin main
```

### 5. Create and Push Tag

Create a version tag and push it to trigger the automated release workflow:

```bash
git tag v1.3.0
git push origin v1.3.0
```

This triggers the GitHub Actions workflow which automatically:
- Builds release binaries for amd64 and arm64
- Runs tests
- Creates .deb packages
- Creates a GitHub release with auto-generated release notes
- Attaches the .deb packages to the release

### 6. Publish to crates.io

```bash
cargo publish
```

Note: Ensure there are no uncommitted or untracked files in the working directory, or use `--allow-dirty` to proceed anyway.

## Version Format

- **Format**: `vMAJOR.MINOR.PATCH`
- **Examples**: `v1.0.0`, `v1.2.3`

## Manual Testing Before Release

Before creating a release, test locally:

```bash
# Run all tests
cargo test

# Check for warnings
cargo clippy

# Check formatting
cargo fmt --check

# Build release binary
cargo build --release
```

## Troubleshooting

### Uncommitted Files Block Publishing

If `cargo publish` complains about uncommitted files:

```bash
# Either remove/gitignore the files
rm path/to/generated/files

# Or publish anyway
cargo publish --allow-dirty
```

### Release Already Exists

If you need to recreate a release:

```bash
# Delete the release (keeps the tag)
gh release delete v1.3.0

# Delete the tag locally and remotely
git tag -d v1.3.0
git push origin --delete v1.3.0

# Recreate and push the tag to trigger the workflow again
git tag v1.3.0
git push origin v1.3.0
```
