//! The HTTP surface: OAuth pages, the JSON API, its `OpenAPI` document, and
//! the single-page UI's static files.

pub mod error;
mod oauth;
mod removals;
mod scan;
pub mod session;
#[cfg(test)]
mod tests;

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use aide::axum::ApiRouter;
use aide::axum::routing::{get_with, post_with};
use aide::openapi::{ApiKeyLocation, Info, OpenApi, SecurityScheme, Tag};
use aide::operation::OperationInput;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderValue, header};
use axum::middleware::from_fn_with_state;
use axum::response::{Html, IntoResponse};
use axum::routing::{any, get};
use axum::{Json, Router};
use cookie::Key;
use indexmap::IndexMap;
use schemars::JsonSchema;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::debug_span;
use url::Url;
use validator::Validate;

use crate::auth::Accounts;
use crate::spotify::Library;
use crate::spotify::retry::RetryPolicy;
use error::AppError;
use session::{CookieCodec, SESSION_COOKIE};

const TAG_AUTHENTICATION: &str = "Authentication";
const TAG_LIBRARY: &str = "Library";
const SESSION_SCHEME: &str = "SessionCookie";

/// Everything the handlers need besides the Spotify clients.
pub struct Settings {
    pub client_id: SecretString,
    /// The origin the browser reaches the app at, e.g.
    /// `https://spoticlean.example`.
    pub public_url: Url,
    pub scan_concurrency: NonZeroUsize,
    pub duration_tolerance: Duration,
    pub retry: RetryPolicy,
}

impl Settings {
    /// Must be registered, byte for byte, in the Spotify developer dashboard.
    pub fn redirect_uri(&self) -> String {
        format!(
            "{}{}",
            self.public_url.as_str().trim_end_matches('/'),
            oauth::CALLBACK_PATH
        )
    }

    /// Browsers drop `Secure` cookies set over plain HTTP, which is how the
    /// app runs on `127.0.0.1`.
    fn secure_cookies(&self) -> bool {
        self.public_url.scheme() == "https"
    }
}

/// Parses `PUBLIC_URL`: an origin, since the app is served at the root.
pub fn parse_public_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("the scheme must be http or https".to_owned());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("must be an origin, with no path, query nor fragment".to_owned());
    }
    Ok(url)
}

struct Shared<L, A> {
    library: L,
    accounts: A,
    settings: Settings,
    cookies: CookieCodec,
}

pub struct AppState<L, A>(Arc<Shared<L, A>>);

impl<L, A> Clone for AppState<L, A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<L, A> AppState<L, A> {
    pub fn new(library: L, accounts: A, settings: Settings, key: &Key) -> Self {
        let cookies = CookieCodec::new(key.clone(), settings.secure_cookies());
        Self(Arc::new(Shared {
            library,
            accounts,
            settings,
            cookies,
        }))
    }

    fn library(&self) -> &L {
        &self.0.library
    }

    fn accounts(&self) -> &A {
        &self.0.accounts
    }

    fn settings(&self) -> &Settings {
        &self.0.settings
    }

    fn cookies(&self) -> &CookieCodec {
        &self.0.cookies
    }
}

/// A JSON body, deserialized then validated, rejected as a problem.
pub struct ValidJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, AppError> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| AppError::InvalidRequest {
                status: rejection.status(),
                detail: rejection.body_text(),
            })?;
        value.validate()?;
        Ok(Self(value))
    }
}

impl<T: JsonSchema> OperationInput for ValidJson<T> {
    fn operation_input(
        ctx: &mut aide::generate::GenContext,
        operation: &mut aide::openapi::Operation,
    ) {
        Json::<T>::operation_input(ctx, operation);
    }
}

/// The whole application. With `static_dir`, paths outside the API serve its
/// files, and `index.html` for any path that is not one, so that the UI's
/// own routes survive a reload; without it, they answer 404.
pub fn router<L, A>(state: AppState<L, A>, static_dir: Option<&Path>) -> Router
where
    L: Library + Send + Sync + 'static,
    A: Accounts + Send + Sync + 'static,
{
    aide::generate::extract_schemas(true);
    let mut api = OpenApi {
        info: Info {
            title: "spoticlean".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            description: Some(
                "Finds duplicates among the Spotify Liked Songs, and removes them.".to_owned(),
            ),
            ..Info::default()
        },
        ..OpenApi::default()
    };
    let authenticated = || from_fn_with_state(state.clone(), session::require::<L, A>);
    let documented = ApiRouter::new()
        .api_route(
            oauth::LOGIN_PATH,
            get_with(oauth::login::<L, A>, oauth::login_docs),
        )
        .api_route(
            oauth::CALLBACK_PATH,
            get_with(oauth::callback::<L, A>, oauth::callback_docs),
        )
        .api_route(
            "/api/v1/session",
            get_with(session::get, session::get_docs)
                .layer(authenticated())
                .delete_with(session::delete::<L, A>, session::delete_docs),
        )
        .api_route(
            "/api/v1/scan",
            get_with(scan::scan::<L, A>, scan::scan_docs).layer(authenticated()),
        )
        .api_route(
            "/api/v1/removals",
            post_with(removals::create::<L, A>, removals::create_docs).layer(authenticated()),
        )
        .finish_api_with(&mut api, |doc| {
            doc.tag(Tag {
                name: TAG_AUTHENTICATION.to_owned(),
                description: Some("Spotify login and the session cookie".to_owned()),
                ..Tag::default()
            })
            .tag(Tag {
                name: TAG_LIBRARY.to_owned(),
                description: Some("Duplicate detection and removal in the Liked Songs".to_owned()),
                ..Tag::default()
            })
            .security_scheme(
                SESSION_SCHEME,
                SecurityScheme::ApiKey {
                    location: ApiKeyLocation::Cookie,
                    name: SESSION_COOKIE.to_owned(),
                    description: Some("Set by `/auth/callback`, encrypted, HttpOnly".to_owned()),
                    extensions: IndexMap::default(),
                },
            )
        });
    let openapi = serde_json::to_vec(&api).expect("the OpenAPI document serializes");

    let router = documented
        .route(
            "/openapi.json",
            get(move || async move {
                (
                    [(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    )],
                    openapi.clone(),
                )
            }),
        )
        .route("/docs", get(|| async { Html(DOCS_PAGE) }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/{*path}", any(not_found))
        .with_state(state);
    let router = match static_dir {
        Some(dir) => router
            .fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => router.fallback(not_found),
    };
    // The default span records the URI, query included, and the callback's
    // query holds the authorization code.
    router.layer(TraceLayer::new_for_http().make_span_with(|request: &Request| {
        debug_span!("request", method = %request.method(), path = request.uri().path())
    }))
}

async fn not_found() -> impl IntoResponse {
    AppError::NotFound
}

const DOCS_PAGE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>spoticlean API</title>
  </head>
  <body>
    <script id="api-reference" data-url="/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;
