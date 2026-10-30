# spoticlean

A web app that finds the duplicates among your Spotify Liked Songs and removes
the ones you do not want.

## Description

Spotify lets the same track end up in your Liked Songs more than once, for
example through different album releases or remasters. spoticlean logs you in
with Spotify, reads your whole library, and groups the tracks that look like
copies of one another, by confidence:

- **certain**: same ISRC, the same recording;
- **probable**: same title and artists once normalized (case, accents,
  `feat.`, remaster and edition suffixes), close durations;
- **variant**: the same song in a different recording (live, acoustic, remix,
  re-recorded, new guests), shown for information.

Each track comes with its album, release date, duration, version and the date
you liked it, so you can tell the original from the reissue; the tracks you
pick are then removed from your Liked Songs.

spoticlean is meant for one person, its owner: it is a server with an HTTP
API and a web UI, not a Spotify client, and it keeps no data of its own. See
[ARCHITECTURE.md](ARCHITECTURE.md) for the design and the reasoning behind it.

## Getting started

### Prerequisites

- Rust, installed with [rustup](https://rustup.rs). The exact toolchain is
  pinned in [rust-toolchain.toml](rust-toolchain.toml) and rustup installs it
  on first build.
- A Spotify app of your own, created in the
  [Spotify developer dashboard](https://developer.spotify.com/dashboard) with
  the Web API enabled, and these redirect URIs, exactly:
  - `http://127.0.0.1:8888/auth/callback` to run it locally;
  - `<PUBLIC_URL>/auth/callback` for each deployment, e.g.
    `https://spoticlean.example/auth/callback`.

  Its client ID is what spoticlean needs; no client secret is used. A
  development-mode app only serves the Spotify accounts added to its *User
  Management* list, which is what decides who can log in.
- [Docker](https://docs.docker.com/get-started/get-docker/), to build the
  image.

### Installation

```sh
git clone https://github.com/leroyguillaume/spoticlean.git
```

```sh
cd spoticlean
```

### Configuration

Every option is an environment variable (or the matching `serve` flag, which
wins).

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `SPOTIFY_CLIENT_ID` | yes | — | Client ID of your Spotify app |
| `SESSION_KEY` | yes | — | Base64 of at least 64 random bytes; encrypts the session cookies, and changing it logs everyone out |
| `PUBLIC_URL` | yes | — | Origin the browser reaches the app at, e.g. `https://spoticlean.example`; cookies are `Secure` when it is HTTPS |
| `BIND_ADDR` | no | `0.0.0.0:8080` | Address the HTTP server listens on |
| `STATIC_DIR` | no | — | Directory of the web UI, served for every path outside the API; without it, only the API is served |
| `SCAN_CONCURRENCY` | no | `4` | Pages of Liked Songs fetched at the same time, 1 to 16 |
| `DURATION_TOLERANCE` | no | `10` | Largest duration gap, in seconds, between two copies of a song |
| `LOG_FILTER` | no | `info` | [`tracing` filter directive](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives), e.g. `spoticlean=debug` |
| `LOG_FORMAT` | no | `text` | Log format on stderr: `text` or `json` |

Under `cargo run`, [.cargo/config.toml](.cargo/config.toml) sets `BIND_ADDR`
and `PUBLIC_URL` for local use; a variable set in your environment wins.

Generate a session key once, and keep it: a new key logs you out.

```sh
openssl rand -base64 64
```

### Usage

```sh
export SPOTIFY_CLIENT_ID=<your-client-id>
```

```sh
export SESSION_KEY=<your-session-key>
```

```sh
cargo run --release -- serve
```

Then log in at <http://127.0.0.1:8888/auth/login>: Spotify asks for read and
modify access to your library, and sends you back to the app. Without
`STATIC_DIR`, the app serves its API only; its reference, from which you can
call it once logged in, is at <http://127.0.0.1:8888/docs>, and the OpenAPI
document at <http://127.0.0.1:8888/openapi.json>.

To log out, call `DELETE /api/v1/session` from the reference, or clear the
`spoticlean_session` cookie.

### Deployment

Build the image:

```sh
docker build -t spoticlean .
```

Run it with the variables above, the secrets passed from your environment
rather than on the command line:

```sh
docker run --rm -p 8888:8080 -e SPOTIFY_CLIENT_ID -e SESSION_KEY -e PUBLIC_URL=http://127.0.0.1:8888 spoticlean
```

The image runs `spoticlean serve` as a non-root user, listens on port 8080,
and checks its own health with `spoticlean healthcheck`; `GET /healthz`
answers `200` for probes. Behind a TLS-terminating proxy, set `PUBLIC_URL` to
the public HTTPS origin.

#### Kubernetes

The [Helm chart](charts/spoticlean/README.md) deploys the image as one
stateless Deployment with a Service and an optional Ingress. It needs
[Helm](https://helm.sh/docs/intro/install/) 3 or 4 and `kubectl` pointed at a
cluster.

Create the namespace, then a Secret holding the client ID and the session key
(keep the key: a new one logs everyone out):

```sh
kubectl create namespace spoticlean
```

```sh
kubectl --namespace spoticlean create secret generic spoticlean --from-literal=SPOTIFY_CLIENT_ID=<your-client-id> --from-literal=SESSION_KEY="$(openssl rand -base64 64 | tr -d '\n')"
```

Describe the deployment in a `values.yaml`; `PUBLIC_URL` is derived from the
ingress host, here `https://spoticlean.example`. The ingress class and the
certificate issuer are yours to name through the annotations:

```yaml
app:
  spotify:
    existingSecret:
      name: spoticlean
  session:
    existingSecret:
      name: spoticlean
  ingress:
    enabled: true
    className: <ingress-class>
    host: spoticlean.example
    annotations:
      cert-manager.io/cluster-issuer: <cluster-issuer>
```

```sh
helm install spoticlean oci://ghcr.io/leroyguillaume/charts/spoticlean --version 0.1.0 --namespace spoticlean -f values.yaml
```

Then register `https://spoticlean.example/auth/callback` as a redirect URI in
the Spotify dashboard. The chart's [README](charts/spoticlean/README.md) lists
every value, among them the TLS settings, resources, extra environment
variables and CA certificates.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
