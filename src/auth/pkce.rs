//! PKCE verifier, challenge and authorization URL (RFC 7636).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use url::Url;

use super::{REDIRECT_URI, SCOPE};

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";

/// A URL-safe random string from `bytes` bytes of OS entropy.
pub fn random_string(bytes: usize) -> Result<SecretString, getrandom::Error> {
    let mut buffer = vec![0; bytes];
    getrandom::fill(&mut buffer)?;
    Ok(URL_SAFE_NO_PAD.encode(buffer).into())
}

/// The `S256` challenge derived from `verifier`.
pub fn challenge(verifier: &SecretString) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose_secret().as_bytes()))
}

/// The Spotify consent page the user is sent to.
pub fn authorize_url(client_id: &SecretString, challenge: &str, state: &str) -> Url {
    Url::parse_with_params(
        AUTHORIZE_URL,
        [
            ("client_id", client_id.expose_secret()),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
            ("state", state),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge),
        ],
    )
    .expect("the authorize URL constant is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc_7636_example() {
        let verifier = SecretString::from("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            challenge(&verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_string_is_a_valid_verifier() {
        let verifier = random_string(32).unwrap();
        let verifier = verifier.expose_secret();
        assert_eq!(verifier.len(), 43);
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(verifier, random_string(32).unwrap().expose_secret());
    }

    #[test]
    fn authorize_url_carries_the_pkce_parameters() {
        let url = authorize_url(
            &SecretString::from("client-123"),
            "the-challenge",
            "the-state",
        );
        assert_eq!(url.host_str(), Some("accounts.spotify.com"));
        assert_eq!(url.path(), "/authorize");
        let params: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let expected: Vec<(String, String)> = [
            ("client_id", "client-123"),
            ("response_type", "code"),
            ("redirect_uri", "http://127.0.0.1:8888/auth/callback"),
            ("scope", "user-library-read"),
            ("state", "the-state"),
            ("code_challenge_method", "S256"),
            ("code_challenge", "the-challenge"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(params, expected);
    }
}
