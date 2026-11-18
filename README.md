# tumpel

The command line client for tumpel. It signs a terminal in to a control plane, applies a tenant config and drives sessions, hosts and connections through the public API.

## Install

Your control plane serves an install script that installs the CLI version it runs:

```sh
curl -fsSL https://<your tumpel host>/v1/releases/install.sh | sh
```

Or install from crates.io:

```sh
cargo install tumpel
```

Each `vX.Y.Z` release on GitHub carries a static Linux binary, `tumpel-x86_64-linux`, and its `.sha256`.

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
