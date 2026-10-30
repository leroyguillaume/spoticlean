# Contributing

Open an issue or a pull request on
[GitHub](https://github.com/leroyguillaume/spoticlean). Every change goes
through a pull request with a green CI.

## Development setup

You need:

- Rust via [rustup](https://rustup.rs); the toolchain pinned in
  [rust-toolchain.toml](rust-toolchain.toml) (with `rustfmt` and `clippy`) is
  installed automatically by the first `cargo` command.
- [Node.js](https://nodejs.org/en/download) with npm, at the version pinned in
  [web/.nvmrc](web/.nvmrc), for the web UI and its hooks.
- [pre-commit](https://pre-commit.com/#install) 4.x. It builds the `actionlint`
  hook with its own Go toolchain, and the `biome-check` hook with Node.js.
- [hadolint](https://github.com/hadolint/hadolint#install),
  [Trivy](https://trivy.dev/latest/getting-started/installation/),
  [Helm](https://helm.sh/docs/intro/install/) 4.x,
  [helm-docs](https://github.com/norwoodj/helm-docs#installation) 1.14,
  [kubeconform](https://github.com/yannh/kubeconform#installation) and
  [ShellCheck](https://github.com/koalaman/shellcheck#installing), which the
  `Dockerfile`, chart and script hooks call from your `PATH`.
- [cargo-edit](https://github.com/killercup/cargo-edit#installation) and
  [jq](https://jqlang.org/download/), only to run
  [`scripts/release.sh`](scripts/release.sh).
- [Docker](https://docs.docker.com/get-started/get-docker/), to build the
  image.

```sh
git clone https://github.com/leroyguillaume/spoticlean.git
```

```sh
cd spoticlean
```

```sh
uv tool install pre-commit
```

**macOS**

```sh
brew install hadolint trivy helm helm-docs kubeconform shellcheck jq cargo-edit
```

**Linux**

```sh
sudo curl -fsSL -o /usr/local/bin/hadolint https://github.com/hadolint/hadolint/releases/download/v2.15.1/hadolint-linux-x86_64
```

```sh
sudo chmod +x /usr/local/bin/hadolint
```

```sh
curl -sfL https://raw.githubusercontent.com/aquasecurity/trivy/main/contrib/install.sh | sudo sh -s -- -b /usr/local/bin v0.76.1
```

Helm, helm-docs, kubeconform and ShellCheck ship static binaries on their
release pages, linked above; ShellCheck and jq are also in most distributions'
package manager, and cargo-edit installs with:

```sh
cargo install cargo-edit
```

```sh
npm ci --prefix web
```

```sh
pre-commit install
```

`cargo test`, `npm test --prefix web` and `pre-commit run --all-files` passing
means the setup is good. See [README.md](README.md#getting-started) for
running the project.

### Working on the web UI

The UI lives in [`web/`](web/): React and TypeScript, built by Vite, linted
and formatted by Biome, tested with Vitest. Its layout and state are described
in [ARCHITECTURE.md](ARCHITECTURE.md#components).

Spotify only sends the browser back to `http://127.0.0.1:8888/auth/callback`,
so in development the Vite dev server takes that address, with hot reload, and
forwards `/api`, `/auth`, `/docs` and `/openapi.json` to the backend, which
moves to port 8889. The browser only ever talks to `127.0.0.1:8888`, so the
cookies and the redirect URI work as in production. In one terminal, with
`SPOTIFY_CLIENT_ID` and `SESSION_KEY` exported as in the
[README](README.md#usage):

```sh
BIND_ADDR=127.0.0.1:8889 cargo run -- serve
```

In another:

```sh
npm run dev --prefix web
```

Then open <http://127.0.0.1:8888>. `PUBLIC_URL` keeps its default from
[.cargo/config.toml](.cargo/config.toml), `http://127.0.0.1:8888`, which is
now the dev server's address.

The UI's client is generated from the backend's OpenAPI document, kept in
[`web/openapi.json`](web/openapi.json): the API's types and their `zod`
schemas land in `web/src/api/generated/`, which is never edited by hand. After
changing a handler or a DTO, `cargo test` fails once and rewrites
`web/openapi.json`; regenerate the client from it, then commit both:

```sh
npm run generate --prefix web
```

## Running the tests

The whole suite, which must be green before a pull request:

```sh
cargo test
```

A single test, or every test of a module:

```sh
cargo test removal_is_batched_by_forty
```

```sh
cargo test matching::
```

All tests are unit tests living next to the code they cover; none needs a
network, a Spotify account or credentials:

- the matching tests run the heuristics on plain `Track` values;
- the fetch and removal tests mock the `Library` trait with `mockall`, and
  run retry delays on tokio's paused clock;
- the API tests in [`src/api/tests.rs`](src/api/tests.rs) build the whole
  router over mocked `Library` and `Accounts`, and send it requests in
  process: login and callback, cookies, the scan's event stream, removals.

The web UI's suite, also required, and a single file of it:

```sh
npm test --prefix web
```

```sh
npm test --prefix web -- src/review/keep.test.ts
```

Its tests run in jsdom, next to the code they cover: the review logic (keep
rules, persisted choices, filters, reasons, dates) on plain values, and the
screens through what they render and how a user clicks them, with `fetch` and
`EventSource` replaced by the fakes in
[`web/src/test/fakes.ts`](web/src/test/fakes.ts). Their fixtures are invented
tracks; never paste a real library into them.

Nothing exercises the real login or the live API: try those by hand with your
own Spotify app, as the [README](README.md#usage) describes.

A new behaviour comes with tests, a fix with a regression test. A
duplicate-detection change comes with a test in
[`src/matching.rs`](src/matching.rs) or
[`src/matching/normalize.rs`](src/matching/normalize.rs) showing the titles
it now groups or keeps apart.

The chart has no test suite of its own: `helm-lint`, `kubeconform` and the
`trivy-chart` scan, run by pre-commit on the chart rendered with the
placeholders in
[`charts/spoticlean/values-lint.yaml`](charts/spoticlean/values-lint.yaml),
are its checks. A new chart value comes with its `# --` annotation in
[`values.yaml`](charts/spoticlean/values.yaml), and `helm-docs` regenerates the
chart's README from it.

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
| `check-added-large-files`, `detect-private-key`, `check-merge-conflict` | no large file (`web/package-lock.json` aside), key or conflict marker committed | remove it |
| `yamllint` | YAML style, configured in [.yamllint.yaml](.yamllint.yaml) | fix the YAML |
| `actionlint` | GitHub Actions workflows | fix the workflow |
| `hadolint` | `Dockerfile` best practices, and ShellCheck on its `RUN` lines | fix the `Dockerfile` |
| `trivy-dockerfile` | `trivy config` misconfigurations in the `Dockerfile` | fix the `Dockerfile` |
| `helm-docs` | the chart's README matches `values.yaml` and its [template](charts/spoticlean/README.md.gotmpl) | re-run, it rewrites the README |
| `helm-lint` | `helm lint --strict` on the chart | fix the chart |
| `helm-kubeconform` | the rendered chart is valid Kubernetes | fix the template |
| `trivy-chart` | `trivy config` misconfigurations in the rendered chart; [`.trivyignore`](.trivyignore) holds the two checks that do not apply to a chart (CPU limit, namespace) and [`.trivy`](.trivy/trusted-registries.json) the trusted registry | fix the chart |
| `shellcheck` | shell scripts | fix the script |
| `biome-check` | formatting and lint of the web UI, configured in [web/biome.json](web/biome.json); its version follows `@biomejs/biome` in [web/package.json](web/package.json) | rewrites what it can, re-run, then fix the lint |
| `web-tsc` | `tsc --noEmit` on the web UI | fix the types |
| `web-openapi-client` | `web/src/api/generated/` matches `web/openapi.json` | re-run, it regenerates the client |
| `cargo-fmt` | `cargo fmt --check` | `cargo fmt` |
| `cargo-clippy` | `cargo clippy -- -D warnings` | fix the lint |
| `no-co-authors` | commit message has no co-author trailer | rewrite the message |

`--no-verify` and `SKIP=` are not the fix: a red hook is a real finding. Fix
the code, or change [.pre-commit-config.yaml](.pre-commit-config.yaml) in the
same pull request and say why.

## Continuous integration

| Workflow | Triggers on | What it does | Reproduce locally |
| --- | --- | --- | --- |
| [`quality.yaml`](.github/workflows/quality.yaml) | every pull request, push to `main` | `pre-commit run --all-files`, `cargo test`, and the web UI's tests and build, in three jobs | `pre-commit run --all-files`, `cargo test`, `npm test --prefix web` and `npm run build --prefix web` |
| [`build.yaml`](.github/workflows/build.yaml) | pull requests and pushes to `main` touching the code, the web UI or the image, manual, called by `release` | builds the image for `amd64` and `arm64`, each on its own runner, and fails on fixable `HIGH`/`CRITICAL` vulnerabilities in it; pushes it to GHCR only when called with `push` (by `release`, or by hand from `main` or a tag, with a version) | `docker build -t spoticlean .` and `trivy image --severity HIGH,CRITICAL --ignore-unfixed spoticlean` |
| [`security.yaml`](.github/workflows/security.yaml) | every pull request, push to `main`, daily, manual | `trivy fs` for fixable `HIGH`/`CRITICAL` vulnerabilities and secrets; on the daily and manual runs, `trivy image` on the latest stable release's published image | `trivy fs .` |
| [`release.yaml`](.github/workflows/release.yaml) | tag `v*` | refuses a tag that disagrees with `Cargo.toml` and `Cargo.lock`, pushes the multi-arch image through `build`, and creates the GitHub release with generated notes (none for a tag with a `-`, such as `-rc1`) | none |
| [`chart.yaml`](.github/workflows/chart.yaml) | tag `chart-*`, manual from `main` with a version | refuses a tag that disagrees with `Chart.yaml` and the README's `helm install --version`, then lints, packages and pushes the chart to `oci://ghcr.io/leroyguillaume/charts` | `helm lint --strict -f charts/spoticlean/values-lint.yaml charts/spoticlean` |

`quality` and `security` are required to merge; `build` only runs when its
inputs change, so it is not a required check. No secret is needed: publishing
uses the workflow's own `GITHUB_TOKEN` with `packages: write`, granted only to
the jobs that push, and never on a pull request.

## Submitting a change

Branch from `main`, rebase on it before opening the pull request. Commit
subjects are lowercase and imperative, like those in `git log --oneline`.

Update the documentation in the same pull request. Give the pull request one
changelog label from [.github/release.yaml](.github/release.yaml) (`feature`,
`fix`, `chore`, ...), plus `breaking` when it breaks something.

## Releasing

The application and the chart are released independently, only with
[`scripts/release.sh`](scripts/release.sh), which refuses a dirty tree, a
branch that is not `main` or is behind `origin`, an existing tag and an
`appVersion` that has no `v*` tag. It bumps the version, regenerates what
derives from it, commits, tags, and asks before pushing: pushing the tag is
what publishes, and declining leaves the commit and the tag local.

```sh
scripts/release.sh 0.2.0
```

That bumps `Cargo.toml` and `Cargo.lock`, commits `release 0.2.0` and tags
`v0.2.0`; [`release.yaml`](.github/workflows/release.yaml) then publishes the
image `ghcr.io/leroyguillaume/spoticlean:0.2.0` and the GitHub release. A
version with a hyphen (`0.3.0-rc1`) publishes the image and no release page.

```sh
scripts/release.sh --chart 0.2.0
```

That sets the chart's `version` and, to the latest `v*` tag unless
`--app-version` says otherwise, its `appVersion`, regenerates the chart's
README, updates the `--version` of the install command in the root README (a
stable version only), commits `release chart 0.2.0` and tags `chart-0.2.0`;
[`chart.yaml`](.github/workflows/chart.yaml) then pushes the chart. The image
the chart deploys is the one tagged with its `appVersion`, so release the
application first. `--dry-run` prints what would happen and changes nothing.
