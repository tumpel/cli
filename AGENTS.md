# Agent rules

Always follow `rules/errors.md` and `rules/coding-standards.md`.

- When writing Rust, follow `rules/rust.md`.
- When writing or changing a test, follow `rules/tests.md`.
- When committing or opening a pull request, follow `rules/commits-and-prs.md`.
- When writing text a user reads, docs, an issue or a PR body, follow `rules/writing.md`.
- Decode a response the CLI only prints with `shown`, so an answer from an older control plane that lacks a newer field still prints.

## Commands

- `npm install` enables the pre-commit hook.
- `npm run check` runs every check. `npm run fix` formats.
- `npm test` runs the Rust tests.
- A `vX.Y.Z` tag releases: the version must match `version` in `Cargo.toml`.
