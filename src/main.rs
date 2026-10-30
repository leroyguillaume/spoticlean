//! spoticlean command-line entry point.

mod auth;
mod matching;
mod report;
mod spotify;

use std::num::NonZeroUsize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use secrecy::SecretString;
use tracing::debug;
use tracing_subscriber::EnvFilter;

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
    #[arg(
        long,
        env = "SPOTICLEAN_LOG_FILTER",
        default_value = "info",
        global = true
    )]
    log_filter: String,

    /// Log output format
    #[arg(long, env = "SPOTICLEAN_LOG_FORMAT", value_enum, default_value_t = LogFormat::Text, global = true)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find duplicates among your Liked Songs
    Saved(SavedArgs),
}

#[derive(clap::Args)]
struct SavedArgs {
    /// Client ID of your Spotify app, whose redirect URI must be
    /// `http://127.0.0.1:8888/auth/callback`
    #[arg(long, env = "SPOTICLEAN_CLIENT_ID", hide_env_values = true)]
    client_id: String,

    /// Number of pages of Liked Songs fetched at the same time
    #[arg(long, env = "SPOTICLEAN_CONCURRENCY", default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
    concurrency: u8,

    /// Largest duration gap, in seconds, between two copies of a song
    #[arg(long, env = "SPOTICLEAN_DURATION_TOLERANCE", default_value_t = 10)]
    duration_tolerance: u64,
}

fn build_filter(directive: &str) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    // `EnvFilter::new` silently drops a directive it cannot parse.
    EnvFilter::try_new(directive)
}

fn unix_now() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("the system clock is before 1970")?
        .as_secs())
}

async fn saved(args: SavedArgs) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("spoticlean/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let cache = auth::TokenCache::new(auth::TokenCache::default_path()?);
    let accounts = auth::SpotifyAccounts::new(http.clone(), SecretString::from(args.client_id));
    let access_token = auth::access_token(&cache, &accounts, unix_now()?).await?;

    let api = spotify::SpotifyApi::new(http, access_token);
    let concurrency =
        NonZeroUsize::new(args.concurrency.into()).context("concurrency must be positive")?;
    let tracks = spotify::fetch::fetch_saved_tracks(
        &api,
        concurrency,
        &spotify::fetch::RetryPolicy::default(),
    )
    .await?;

    let groups = matching::find_groups(&tracks, Duration::from_secs(args.duration_tolerance));
    debug!(groups = groups.len(), "matched liked songs");
    print!("{}", report::render(&groups, tracks.len()));
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
        Command::Saved(args) => saved(args).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(
            ["spoticlean", "saved", "--client-id", "id"]
                .iter()
                .chain(args),
        )
    }

    #[test]
    fn build_filter_accepts_valid_directive() {
        assert!(build_filter("spoticlean=debug,warn").is_ok());
    }

    #[test]
    fn build_filter_rejects_invalid_directive() {
        assert!(build_filter("spoticlean=nope").is_err());
    }

    #[test]
    fn cli_defaults() {
        let cli = parse(&[]).unwrap();
        assert_eq!(cli.log_filter, "info");
        assert!(matches!(cli.log_format, LogFormat::Text));
        let Command::Saved(args) = cli.command;
        assert_eq!(args.concurrency, 4);
        assert_eq!(args.duration_tolerance, 10);
    }

    #[test]
    fn cli_rejects_out_of_range_concurrency() {
        assert!(parse(&["--concurrency", "0"]).is_err());
        assert!(parse(&["--concurrency", "17"]).is_err());
        assert!(parse(&["--concurrency", "16"]).is_ok());
    }

    #[test]
    fn help_never_shows_the_client_id_value() {
        let mut command = Cli::command();
        let saved = command.find_subcommand_mut("saved").unwrap();
        let client_id = saved
            .get_arguments()
            .find(|arg| arg.get_id() == "client_id")
            .unwrap();
        assert!(client_id.is_hide_env_values_set());
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }
}
