# Contributing

Open an issue or a pull request on
[GitHub](https://github.com/leroyguillaume/spoticlean). Every change goes
through a pull request with a green CI.

## Development setup

You need:

- Rust via [rustup](https://rustup.rs); the toolchain pinned in
  [rust-toolchain.toml](rust-toolchain.toml) (with `rustfmt` and `clippy`) is
  installed automatically by the first `cargo` command.
- [pre-commit](https://pre-commit.com/#install) 4.x. It builds the `actionlint`
  hook with its own Go toolchain, so nothing else needs installing.

```sh
git clone https://github.com/leroyguillaume/spoticlean.git
```

```sh
cd spoticlean
```

```sh
uv tool install pre-commit
```

```sh
pre-commit install
```

`cargo test` passing means the setup is good. See
[README.md](README.md#getting-started) for running the project.

## Running the tests

The whole suite, which must be green before a pull request:

```sh
cargo test
```

A single test, or every test of a module:

```sh
cargo test build_filter_rejects_invalid_directive
```

```sh
cargo test matching::
```

All tests are unit tests living next to the code they cover; none needs a
network, a Spotify account or credentials. The Spotify and accounts clients
are traits mocked with `mockall`, retry delays run on tokio's paused clock, and
the callback listener tests bind an ephemeral port on `127.0.0.1`. Nothing
exercises the real login or the live API: try those by hand with your own
Spotify app, as the [README](README.md#usage) describes.

A new behaviour comes with tests, a fix with a regression test. A
duplicate-detection change comes with a test in
[`src/matching.rs`](src/matching.rs) or
[`src/matching/normalize.rs`](src/matching/normalize.rs) showing the titles
it now groups or keeps apart.

## Pre-commit hooks

`pre-commit install` sets up the hooks; they must pass before a commit is
pushed. The same hooks run in CI, so skipping them locally only moves the
failure somewhere slower.

```sh
pre-commit run --all-files
```

```sh
pre-commit run <hook-id> --all-files
```

| Hook | What it checks | Fix |
| --- | --- | --- |
| `trailing-whitespace`, `end-of-file-fixer` | whitespace and final newline | rewrites the files, re-run |
| `check-yaml`, `check-toml` | files parse | fix the syntax |
| `check-added-large-files`, `detect-private-key`, `check-merge-conflict` | no large file, key or conflict marker committed | remove it |
| `yamllint` | YAML style, configured in [.yamllint.yaml](.yamllint.yaml) | fix the YAML |
| `actionlint` | GitHub Actions workflows | fix the workflow |
| `cargo-fmt` | `cargo fmt --check` | `cargo fmt` |
| `cargo-clippy` | `cargo clippy -- -D warnings` | fix the lint |
| `no-co-authors` | commit message has no co-author trailer | rewrite the message |

`--no-verify` and `SKIP=` are not the fix: a red hook is a real finding. Fix
the code, or change [.pre-commit-config.yaml](.pre-commit-config.yaml) in the
same pull request and say why.

## Continuous integration

| Workflow | Triggers on | What it does | Reproduce locally |
| --- | --- | --- | --- |
| [`quality.yaml`](.github/workflows/quality.yaml) | every pull request, push to `main` | `pre-commit run --all-files`, then `cargo test` | `pre-commit run --all-files` and `cargo test` |
| [`security.yaml`](.github/workflows/security.yaml) | every pull request, push to `main`, daily, manual | `trivy fs` for fixable `HIGH`/`CRITICAL` vulnerabilities and secrets | `trivy fs .` |

Both workflows are required to merge. No secret is needed.

## Submitting a change

Branch from `main`, rebase on it before opening the pull request. Commit
subjects are lowercase and imperative, like those in `git log --oneline`.

Update the documentation in the same pull request. Give the pull request one
changelog label from [.github/release.yaml](.github/release.yaml) (`feature`,
`fix`, `chore`, ...), plus `breaking` when it breaks something.
