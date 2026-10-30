//! spoticlean command-line entry point.

use clap::Parser;
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
    #[arg(long, env = "SPOTICLEAN_LOG_FILTER", default_value = "info")]
    log_filter: String,

    /// Log output format
    #[arg(long, env = "SPOTICLEAN_LOG_FORMAT", value_enum, default_value_t = LogFormat::Text)]
    log_format: LogFormat,
}

fn build_filter(directive: &str) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    // `EnvFilter::new` silently drops a directive it cannot parse.
    EnvFilter::try_new(directive)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let fmt = tracing_subscriber::fmt()
        .with_env_filter(build_filter(&cli.log_filter)?)
        .with_writer(std::io::stderr);
    match cli.log_format {
        LogFormat::Text => fmt.init(),
        LogFormat::Json => fmt.json().init(),
    }
    debug!(filter = %cli.log_filter, "logging initialised");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let cli = Cli::parse_from(["spoticlean"]);
        assert_eq!(cli.log_filter, "info");
        assert!(matches!(cli.log_format, LogFormat::Text));
    }
}
