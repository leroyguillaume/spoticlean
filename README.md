# spoticlean

A command-line tool that finds duplicates among your Spotify Liked Songs.

## Description

Spotify lets the same track end up in your Liked Songs more than once, for
example through different album releases or re-uploads. spoticlean is a
personal command-line tool meant to spot those duplicates.

It reads your library; it is not a Spotify client and does not play music.

## Getting started

### Prerequisites

- Rust, installed with [rustup](https://rustup.rs). The exact toolchain is
  pinned in [rust-toolchain.toml](rust-toolchain.toml) and rustup installs it
  on first build.

### Installation

```sh
git clone https://github.com/leroyguillaume/spoticlean.git
```

```sh
cd spoticlean
```

```sh
cargo install --path .
```

### Configuration

Every option can be set with a flag or with an environment variable; the flag
wins.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `SPOTICLEAN_LOG_FILTER` | no | `info` | [`tracing` filter directive](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives), e.g. `spoticlean=debug` |
| `SPOTICLEAN_LOG_FORMAT` | no | `text` | Log format on stderr: `text` or `json` |

### Usage

```sh
spoticlean --help
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
