//! Token cache file, readable by its owner only.

use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use etcetera::BaseStrategy as _;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::Token;

#[derive(Serialize, Deserialize)]
struct CachedToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    scope: String,
}

pub struct TokenCache {
    path: PathBuf,
}

impl TokenCache {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `$XDG_CONFIG_HOME/spoticlean/token.json`, `~/.config` by default, on
    /// macOS as well.
    pub fn default_path() -> anyhow::Result<PathBuf> {
        let strategy =
            etcetera::choose_base_strategy().context("cannot find the home directory")?;
        Ok(strategy.config_dir().join("spoticlean").join("token.json"))
    }

    pub fn load(&self) -> anyhow::Result<Option<Token>> {
        let content = match fs::read(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                debug!(path = %self.path.display(), "no cached token");
                return Ok(None);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", self.path.display()));
            }
        };
        let cached: CachedToken = serde_json::from_slice(&content)
            .with_context(|| format!("cannot parse {}", self.path.display()))?;
        debug!(path = %self.path.display(), expires_at = cached.expires_at, "loaded cached token");
        Ok(Some(Token {
            access: cached.access_token.into(),
            refresh: cached.refresh_token.map(SecretString::from),
            expires_at: cached.expires_at,
            scope: cached.scope,
        }))
    }

    /// Writes a sibling file then renames it over the cache, so that an
    /// interrupted write never leaves a truncated cache behind.
    pub fn save(&self, token: &Token) -> anyhow::Result<()> {
        let dir = self
            .path
            .parent()
            .context("the token cache path has no parent directory")?;
        create_private_dir(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let cached = CachedToken {
            access_token: token.access.expose_secret().to_owned(),
            refresh_token: token
                .refresh
                .as_ref()
                .map(|refresh| refresh.expose_secret().to_owned()),
            expires_at: token.expires_at,
            scope: token.scope.clone(),
        };
        let content = serde_json::to_vec(&cached)?;
        let temporary = self.path.with_extension("json.tmp");
        write_private(&temporary, &content)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        fs::rename(&temporary, &self.path)
            .with_context(|| format!("cannot write {}", self.path.display()))?;
        debug!(path = %self.path.display(), "saved token");
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

fn write_private(path: &Path, content: &[u8]) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> Token {
        Token {
            access: "access-value".into(),
            refresh: Some("refresh-value".into()),
            expires_at: 1_700_000_000,
            scope: "user-library-read".to_owned(),
        }
    }

    #[test]
    fn missing_file_is_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TokenCache::new(dir.path().join("token.json"));
        assert!(cache.load().unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TokenCache::new(dir.path().join("nested").join("token.json"));
        cache.save(&token()).unwrap();
        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.access.expose_secret(), "access-value");
        assert_eq!(loaded.refresh.unwrap().expose_secret(), "refresh-value");
        assert_eq!(loaded.expires_at, 1_700_000_000);
        assert_eq!(loaded.scope, "user-library-read");
        assert!(!dir.path().join("nested").join("token.json.tmp").exists());
    }

    #[test]
    fn save_overwrites_the_previous_token() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TokenCache::new(dir.path().join("token.json"));
        cache.save(&token()).unwrap();
        let mut newer = token();
        newer.access = "newer-access".into();
        newer.refresh = None;
        cache.save(&newer).unwrap();
        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.access.expose_secret(), "newer-access");
        assert!(loaded.refresh.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cache_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("spoticlean");
        let cache = TokenCache::new(config.join("token.json"));
        cache.save(&token()).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&config.join("token.json")), 0o600);
        assert_eq!(mode(&config), 0o700);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        fs::write(&path, "not json").unwrap();
        assert!(TokenCache::new(path).load().is_err());
    }

    #[test]
    fn default_path_ends_with_the_app_directory() {
        let path = TokenCache::default_path().unwrap();
        assert!(path.ends_with("spoticlean/token.json"));
    }
}
