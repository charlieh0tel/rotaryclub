# Gemini Instructions for Rotary Club Project

- Always run `cargo fmt` after changes and before commits.
- Always run `cargo clippy` after major changes and always before commits.
- All tests must pass before committing (`cargo test`).
- Use relative imports.
- Do not add trivial, obvious or redundant comments.
- Do not add Gemini attribution to commit messages.
- PRs should generally be comprised of one functional change; suggest
  making a commit before moving onto something unrelated.
- When work completes a tracked item, update `TODO.md` in the same commit
  by marking that item done.
