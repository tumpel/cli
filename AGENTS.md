# Agent rules

Always follow `rules/errors.md` and `rules/coding-standards.md`.

- When writing Rust, follow `rules/rust.md`.
- When writing or changing a test, follow `rules/tests.md`.
- When committing or opening a pull request, follow `rules/commits-and-prs.md`.
- When writing text a user reads, docs, an issue or a PR body, follow `rules/writing.md`.
- Decode a response the CLI only prints with `shown`, so an answer from an older control plane that lacks a newer field still prints.
- Consume a tumpel/protocol release candidate as a Cargo git dependency on `https://github.com/tumpel/protocol`, pinned to its exact `vX.Y.Z-rc.N` tag. The `promote` workflow switches it to the exact crates.io `X.Y.Z` version before `cargo publish`.

## Commands

- `npm install` enables the pre-commit hook.
- `npm run check` runs every check. `npm run fix` formats.
- `npm test` runs the Rust tests.
- Releases are mechanical; never bump the version in `Cargo.toml`, tag or publish by hand. A merge to `main` that changes a shipped file publishes a GitHub pre-release `vX.Y.Z-rc.N`, versioned from the conventional commit types (breaking major, `feat` minor, `fix` and `perf` patch). Only the manual `promote` workflow ships a release candidate to crates.io as `vX.Y.Z`. A rerun releases the same version.
