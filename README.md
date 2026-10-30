# spoticlean

A command-line tool that finds duplicates among your Spotify Liked Songs.

## Description

Spotify lets the same track end up in your Liked Songs more than once, for
example through different album releases or remasters. `spoticlean saved`
reads your whole library and lists the tracks that look like copies of one
another, grouped by confidence:

- **certain**: same ISRC, the same recording;
- **probable**: same title and artist once normalized (case, accents,
  `feat.`, remaster and edition suffixes), close durations;
- **variant**: same song, but a different recording (live, acoustic, remix,
  re-recorded), shown for information only.

Each track comes with its album, release date, duration and the date you saved
it, so you can tell the original from the reissue.

spoticlean only reads your library: it removes nothing, and it is not a
Spotify client. See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and the
reasoning behind it.

## Getting started

### Prerequisites

- Rust, installed with [rustup](https://rustup.rs). The exact toolchain is
  pinned in [rust-toolchain.toml](rust-toolchain.toml) and rustup installs it
  on first build.
- A Spotify app of your own, created in the
  [Spotify developer dashboard](https://developer.spotify.com/dashboard) with
  the Web API enabled and this redirect URI, exactly:
  `http://127.0.0.1:8888/auth/callback`. Its client ID is what spoticlean
  needs; no client secret is used. A development-mode app only serves the
  Spotify accounts added to its *User Management* list.

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
| `SPOTICLEAN_CLIENT_ID` | yes, for `saved` | — | Client ID of your Spotify app |
| `SPOTICLEAN_CONCURRENCY` | no | `4` | Pages of Liked Songs fetched at the same time, 1 to 16 |
| `SPOTICLEAN_DURATION_TOLERANCE` | no | `10` | Largest duration gap, in seconds, between two copies of a song |
| `SPOTICLEAN_LOG_FILTER` | no | `info` | [`tracing` filter directive](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives), e.g. `spoticlean=debug` |
| `SPOTICLEAN_LOG_FORMAT` | no | `text` | Log format on stderr: `text` or `json` |

The Spotify tokens are cached in `$XDG_CONFIG_HOME/spoticlean/token.json`,
that is `~/.config/spoticlean/token.json` by default on macOS and Linux,
readable by you only.

### Usage

```sh
export SPOTICLEAN_CLIENT_ID=<your-client-id>
```

```sh
spoticlean saved
```

The first run opens your browser on the Spotify consent page (the terminal
prints a local URL to open by hand if the browser does not start), asking for
read access to your library. Once you accept, the browser shows a confirmation
you can close, and the token is cached: later runs refresh it on their own and
only open the browser again when it can no longer be refreshed. Port 8888 must
be free during that login.

The report goes to stdout and the logs to stderr, so the report can be saved
on its own:

```sh
spoticlean saved > duplicates.txt
```

To log in again from scratch, delete the cached token:

```sh
rm ~/.config/spoticlean/token.json
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
