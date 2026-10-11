# Mirrors what CI checks. ruff goes through uv so it runs the version
# pinned in uv.lock, not whatever ruff is on PATH.
RUFF := uv run --frozen ruff

.PHONY: lint fmt

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	$(RUFF) check .
	$(RUFF) format --check .

fmt:
	cargo fmt --all
	$(RUFF) format .
