//! spoticlean server entry point.

mod api;
mod auth;
mod matching;
mod spotify;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use clap::{Parser, Subcommand};
use cookie::Key;
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Clone, Copy, clap::ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// `tracing` filter directive (e.g. `info`, `spoticlean=debug`)
    ///
    /// Syntax: <https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives>
    #[arg(long, env = "LOG_FILTER", default_value = "info", global = true)]
    log_filter: String,

    /// Log output format
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value_t = LogFormat::Text, global = true)]
    log_format: LogFormat,

    /// Address the HTTP server listens on
    #[arg(long, env = "BIND_ADDR", default_value_t = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080)), global = true)]
    bind_addr: SocketAddr,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the web app
    Serve(ServeArgs),
    /// Exit 0 when the server listening on `BIND_ADDR` answers its health
    /// check, 1 otherwise
    Healthcheck,
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Origin the browser reaches the app at, e.g. `https://spoticlean.example`;
    /// `<origin>/auth/callback` must be a redirect URI of the Spotify app
    #[arg(long, env = "PUBLIC_URL", value_parser = api::parse_public_url)]
    public_url: Url,

    /// Client ID of the Spotify app
    #[arg(long, env = "SPOTIFY_CLIENT_ID", hide_env_values = true)]
    spotify_client_id: String,

    /// Base64 of at least 64 random bytes, which encrypt and sign the session
    /// cookies, e.g. `openssl rand -base64 64`
    #[arg(long, env = "SESSION_KEY", hide_env_values = true)]
    session_key: String,

    /// Directory holding the web UI, served for every path outside the API
    #[arg(long, env = "STATIC_DIR")]
    static_dir: Option<PathBuf>,

    /// Number of pages of Liked Songs fetched at the same time
    #[arg(long, env = "SCAN_CONCURRENCY", default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
    scan_concurrency: u8,

    /// Largest duration gap, in seconds, between two copies of a song
    #[arg(long, env = "DURATION_TOLERANCE", default_value_t = 10)]
    duration_tolerance: u64,
}

fn build_filter(directive: &str) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    // `EnvFilter::new` silently drops a directive it cannot parse.
    EnvFilter::try_new(directive)
}

/// Never echoes the value: clap's own parse errors would.
fn parse_session_key(value: &SecretString) -> anyhow::Result<Key> {
    let bytes = STANDARD
        .decode(value.expose_secret().trim())
        .map_err(|_| anyhow::anyhow!("SESSION_KEY is not valid base64"))?;
    Key::try_from(bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("SESSION_KEY must decode to at least 64 bytes"))
}

fn check_static_dir(dir: &std::path::Path) -> anyhow::Result<()> {
    if !dir.is_dir() {
        bail!("STATIC_DIR {} is not a directory", dir.display());
    }
    if !dir.join("index.html").is_file() {
        warn!(dir = %dir.display(), "no index.html in the static directory, the UI routes answer 404");
    }
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt()).expect("SIGINT handler");
    let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = interrupt.recv() => {},
        _ = terminate.recv() => {},
    }
    info!("shutting down, draining open connections");
}

async fn serve(bind_addr: SocketAddr, args: ServeArgs) -> anyhow::Result<()> {
    let key = parse_session_key(&SecretString::from(args.session_key))?;
    if let Some(dir) = &args.static_dir {
        check_static_dir(dir)?;
    } else {
        info!("no STATIC_DIR, serving the API only");
    }
    let http = reqwest::Client::builder()
        .user_agent(concat!("spoticlean/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let client_id = SecretString::from(args.spotify_client_id);
    let settings = api::Settings {
        client_id: client_id.clone(),
        public_url: args.public_url,
        scan_concurrency: NonZeroUsize::new(args.scan_concurrency.into())
            .context("SCAN_CONCURRENCY must be positive")?,
        duration_tolerance: Duration::from_secs(args.duration_tolerance),
        retry: spotify::retry::RetryPolicy::default(),
    };
    debug!(redirect_uri = %settings.redirect_uri(), "Spotify redirect URI");
    let state = api::AppState::new(
        spotify::SpotifyApi::new(http.clone()),
        auth::SpotifyAccounts::new(http, client_id),
        settings,
        &key,
    );
    let app = api::router(state, args.static_dir.as_deref());

    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("cannot listen on {bind_addr}"))?;
    info!(addr = %bind_addr, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("the HTTP server failed")?;
    info!("stopped");
    Ok(())
}

/// The address to reach a server listening on `bind_addr` from the same host.
fn local_addr(bind_addr: SocketAddr) -> SocketAddr {
    let ip = match bind_addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind_addr.port())
}

async fn healthcheck(bind_addr: SocketAddr) -> anyhow::Result<()> {
    let url = format!("http://{}/healthz", local_addr(bind_addr));
    debug!(%url, "checking health");
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("no answer from {url}"))?;
    if !response.status().is_success() {
        bail!("{url} answered HTTP {}", response.status());
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let fmt = tracing_subscriber::fmt()
        .with_env_filter(build_filter(&cli.log_filter)?)
        .with_writer(std::io::stderr);
    match cli.log_format {
        LogFormat::Text => fmt.init(),
        LogFormat::Json => fmt.json().init(),
    }
    debug!(filter = %cli.log_filter, "logging initialised");
    match cli.command {
        Command::Serve(args) => serve(cli.bind_addr, args).await,
        Command::Healthcheck => healthcheck(cli.bind_addr).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse_with_url(public_url: &str, args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(
            [
                "spoticlean",
                "serve",
                "--public-url",
                public_url,
                "--spotify-client-id",
                "id",
                "--session-key",
                "key",
            ]
            .iter()
            .chain(args),
        )
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        parse_with_url("https://spoticlean.example", args)
    }

    #[test]
    fn build_filter_accepts_valid_directive() {
        assert!(build_filter("spoticlean=debug,warn").is_ok());
    }

    #[test]
    fn build_filter_rejects_invalid_directive() {
        assert!(build_filter("spoticlean=nope").is_err());
    }

    /// Read from the definition rather than parsed: `cargo test` gets the
    /// `.cargo/config.toml` environment, which overrides some defaults.
    #[test]
    fn cli_defaults() {
        let command = Cli::command();
        let mut defaults: Vec<(String, String)> = command
            .get_arguments()
            .map(|arg| (arg.get_id().to_string(), arg.get_default_values().to_vec()))
            .chain(
                command
                    .find_subcommand("serve")
                    .unwrap()
                    .get_arguments()
                    .map(|arg| (arg.get_id().to_string(), arg.get_default_values().to_vec())),
            )
            .filter_map(|(id, values)| Some((id, values.first()?.to_string_lossy().into_owned())))
            .collect();
        defaults.sort();
        let expected: Vec<(String, String)> = [
            ("bind_addr", "0.0.0.0:8080"),
            ("duration_tolerance", "10"),
            ("log_filter", "info"),
            ("log_format", "text"),
            ("scan_concurrency", "4"),
        ]
        .into_iter()
        .map(|(id, value)| (id.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(defaults, expected);
    }

    #[test]
    fn cli_rejects_out_of_range_concurrency() {
        assert!(parse(&["--scan-concurrency", "0"]).is_err());
        assert!(parse(&["--scan-concurrency", "17"]).is_err());
        assert!(parse(&["--scan-concurrency", "16"]).is_ok());
    }

    #[test]
    fn cli_rejects_a_public_url_with_a_path() {
        assert!(parse_with_url("https://example.com/app", &[]).is_err());
        assert!(parse_with_url("https://example.com/?a=b", &[]).is_err());
        assert!(parse_with_url("ftp://example.com", &[]).is_err());
        assert!(parse_with_url("http://127.0.0.1:8888", &[]).is_ok());
        assert!(parse_with_url("https://example.com/", &[]).is_ok());
    }

    #[test]
    fn healthcheck_needs_no_server_settings() {
        let cli = Cli::try_parse_from(["spoticlean", "healthcheck"]).unwrap();
        assert!(matches!(cli.command, Command::Healthcheck));
    }

    #[test]
    fn help_never_shows_secret_values() {
        let mut command = Cli::command();
        let serve = command.find_subcommand_mut("serve").unwrap();
        for id in ["spotify_client_id", "session_key"] {
            let arg = serve
                .get_arguments()
                .find(|arg| arg.get_id() == id)
                .unwrap();
            assert!(arg.is_hide_env_values_set(), "{id}");
        }
    }

    #[test]
    fn session_key_needs_64_bytes_of_base64() {
        let short = SecretString::from(STANDARD.encode([1; 32]));
        assert!(parse_session_key(&short).is_err());
        assert!(parse_session_key(&SecretString::from("not base64!")).is_err());
        let long = SecretString::from(format!("{}\n", STANDARD.encode([1; 64])));
        assert!(parse_session_key(&long).is_ok());
    }

    #[test]
    fn local_addr_targets_loopback_for_wildcards() {
        assert_eq!(
            local_addr("0.0.0.0:8080".parse().unwrap()),
            "127.0.0.1:8080".parse().unwrap()
        );
        assert_eq!(
            local_addr("[::]:8080".parse().unwrap()),
            "[::1]:8080".parse().unwrap()
        );
        assert_eq!(
            local_addr("127.0.0.1:8888".parse().unwrap()),
            "127.0.0.1:8888".parse().unwrap()
        );
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }
}
