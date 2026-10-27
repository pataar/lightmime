# Contributing

Thanks for helping out. Bug reports, ideas and pull requests are all welcome. For anything bigger than a small fix,
open an issue first so we can agree on the approach before you write the code.

## Setup

The toolchain is pinned in `rust-toolchain.toml`; rustup picks it up. You also need the development files of
GStreamer, OpenSSL and xkbcommon; [Install → From source](https://lightmime.pataar.nl/install.html#from-source) lists
the packages per distribution, and CI installs the ones in `packaging/linux/build-deps-apt.txt`.

```sh
cargo run -- gui              # the window and tray, with your real config
cargo run -- --config /tmp/lightmime.toml gui   # or with a scratch one
```

[AGENTS.md](AGENTS.md) describes the architecture, the protocol and the Home Assistant contract.

## Before you open a pull request

All of these must pass; CI runs the same set:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy -p lightmime --no-default-features -- -D warnings
cargo test --all-features
```

- New logic comes with a test where it can have one; colour mapping and parsing can, streaming to a bridge can't.
- Every new dependency gets a one-line comment in `Cargo.toml` saying why it is needed, when that is not obvious.
- User-facing docs live in `docs/src/` (an [mdbook](https://rust-lang.github.io/mdBook/); `mdbook serve docs` previews
  it). Update them with any change to commands, config keys, the window or Home Assistant entities.
- After changing the window, refresh the screenshots in the docs:
  `cargo test -p lightmime-gui screenshots -- --ignored` (needs a GPU or a software Vulkan driver).
- Home Assistant entity keys and unique ids are a public API: renaming one creates a new entity for every user.
- Keep a pull request to one change.

## Testing with real lights

There is no bridge simulator. `lightmime test` checks pairing and streaming without screen capture;
`lightmime run --start` exercises the whole pipeline. Say in the pull request which bridge and desktop you tested on.

## Commit messages and pull request titles

Use [Conventional Commits](https://www.conventionalcommits.org): `feat: …`, `fix: …`, `docs: …`, `chore: …`. Pull
requests are squash-merged, so the title becomes the commit on `main`, and release-plz uses it for the changelog and
the version bump.

## Privacy

Never paste bridge keys, MQTT passwords or your config file into issues, tests or logs.

## Licence

By contributing, you agree that your contribution is dual-licensed under MIT and Apache-2.0, like the rest of the
project, without any additional terms.
