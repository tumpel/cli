<p align="center"><img src="doc/assets/mascot.svg" alt="The tumpel salamander" width="160"></p>

# tumpel

The command line client for tumpel. It signs a terminal in to a control plane, applies a tenant config and drives sessions, hosts and connections through the public API.

## Install

Your control plane serves an install script that installs the CLI version it runs:

```sh
curl -fsSL https://<your tumpel host>/v1/releases/install.sh | sh
```

Each release on GitHub carries a static Linux binary, `tumpel-x86_64-linux`, and its `.sha256`, at `https://github.com/tumpel/cli/releases/download/v<version>/tumpel-x86_64-linux`.

## Releases

Only a change to the crate makes a new version: its sources, `Cargo.toml`, `Cargo.lock` or the build script. Docs, tests, CI and the Claude Code plugin never do. The conventional commits that changed those files since the latest stable tag pick the next version: a breaking change bumps major, `feat` bumps minor, and `fix` or `perf` bumps patch. Other types ride along with the next release.

- Release candidate: a push to `main` with such a change publishes the GitHub pre-release `vX.Y.Z-rc.N` with the binary, where `N` counts the commits since the latest stable tag.
- Stable release: a person runs the `promote` workflow and names one release candidate. It tags that commit `vX.Y.Z` and publishes the GitHub release as Latest. To ship several changes together, merge them and promote the last release candidate.

A failed run is rerun as it is and releases the same version: a rerun reuses the stable tag and finishes a draft release, and a published release is never overwritten.

## Sign in

```sh
tumpel login --endpoint https://<your tumpel host>
tumpel whoami
```

`tumpel --help` lists every command.

## Claude Code plugin

The `tumpel` plugin tells Claude Code when a turn ends, a session goes idle, a host goes offline or a prebuild finishes. It runs `tumpel events follow` for the whole session, so install the CLI and sign in first. Add it from Claude Code:

```text
/plugin marketplace add tumpel/cli
/plugin install tumpel@tumpel
```

The plugin starts in interactive sessions only.

## Development

```sh
npm install
npm run check
npm test
```

## Licence

Apache-2.0. See `LICENSE` and `NOTICE`.

## Server compatibility

The CLI prints one update message when the server sends a response it does not recognise. Printable JSON still appears on stdout. When the command cannot continue, it exits with code 3; other command failures use code 1 and argument errors use code 2.
