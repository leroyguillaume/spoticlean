use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt as _;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::*;
use crate::auth::{AccountsError, Grant, MockAccounts};
use crate::matching::Track;
use crate::spotify::{ApiError, MockLibrary, SavedTracksPage, ready};
use session::{Session, unix_now};

const PUBLIC_URL: &str = "https://spoticlean.example";

fn key() -> Key {
    Key::from(&[7; 64])
}

fn settings() -> Settings {
    Settings {
        client_id: SecretString::from("client-123"),
        public_url: Url::parse(PUBLIC_URL).unwrap(),
        scan_concurrency: NonZeroUsize::new(4).unwrap(),
        duration_tolerance: Duration::from_secs(10),
        retry: RetryPolicy::default(),
    }
}

fn app_with_static(
    library: MockLibrary,
    accounts: MockAccounts,
    static_dir: Option<&Path>,
) -> Router {
    router(
        AppState::new(library, accounts, settings(), &key()),
        static_dir,
    )
}

fn app(library: MockLibrary, accounts: MockAccounts) -> Router {
    app_with_static(library, accounts, None)
}

fn codec() -> CookieCodec {
    CookieCodec::new(key(), true)
}

fn session(expires_in: i64, refresh: Option<&str>) -> Session {
    Session {
        access: SecretString::from("access-token"),
        refresh: refresh.map(SecretString::from),
        expires_at: unix_now().saturating_add_signed(expires_in),
        scope: crate::auth::SCOPES.to_owned(),
    }
}

/// The `name=value` part of a `Set-Cookie` header, as a browser sends it back.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_owned()
}

fn session_cookie(expires_in: i64) -> String {
    cookie_pair(
        codec()
            .session(&session(expires_in, Some("refresh-token")))
            .to_str()
            .unwrap(),
    )
}

fn valid_session_cookie() -> String {
    session_cookie(3600)
}

fn set_cookies(response: &Response) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

fn request(method: Method, uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

fn json_request(uri: &str, cookie: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_json(response: Response) -> Value {
    serde_json::from_str(&body_text(response).await).unwrap()
}

fn location(response: &Response) -> &str {
    response.headers()[header::LOCATION].to_str().unwrap()
}

fn grant(access: &str, refresh: Option<&str>) -> Grant {
    Grant {
        access: SecretString::from(access),
        refresh: refresh.map(SecretString::from),
        expires_in: 3600,
        scope: crate::auth::SCOPES.to_owned(),
    }
}

/// Runs `/auth/login` and returns the login cookie, and the `state` and PKCE
/// challenge sent to Spotify.
async fn start_login(app: &Router) -> (String, String, String) {
    let response = app
        .clone()
        .oneshot(request(Method::GET, "/auth/login", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let url = Url::parse(location(&response)).unwrap();
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap()
    };
    let cookie = cookie_pair(&set_cookies(&response)[0]);
    (cookie, param("state"), param("code_challenge"))
}

#[tokio::test]
async fn login_redirects_to_spotify_with_pkce() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let response = app
        .oneshot(request(Method::GET, "/auth/login", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let url = Url::parse(location(&response)).unwrap();
    assert_eq!(url.host_str(), Some("accounts.spotify.com"));
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "client-123");
    assert_eq!(
        params["redirect_uri"],
        "https://spoticlean.example/auth/callback"
    );
    assert_eq!(params["scope"], "user-library-read user-library-modify");
    assert_eq!(params["code_challenge_method"], "S256");
    assert_ne!(params["state"], "");

    let cookies = set_cookies(&response);
    assert_eq!(cookies.len(), 1);
    let cookie = &cookies[0];
    assert!(cookie.starts_with("spoticlean_login="));
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/auth"] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing in {cookie}"
        );
    }
    assert!(!cookie.contains(&params["state"]), "the state is readable");
}

#[tokio::test]
async fn callback_exchanges_the_code_and_sets_the_session() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let mut accounts = MockAccounts::new();
    let expected = Arc::clone(&challenge);
    accounts
        .expect_exchange_code()
        .withf(move |code, verifier, redirect_uri| {
            code.expose_secret() == "the-code"
                && redirect_uri == "https://spoticlean.example/auth/callback"
                && crate::auth::pkce::challenge(verifier) == *expected.lock().unwrap()
        })
        .times(1)
        .returning(|_, _, _| ready(Ok(grant("access-token", Some("refresh-token")))));
    let app = app(MockLibrary::new(), accounts);
    let (login_cookie, state, sent_challenge) = start_login(&app).await;
    *challenge.lock().unwrap() = sent_challenge;

    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/auth/callback?code=the-code&state={state}"),
            Some(&login_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/");
    let cookies = set_cookies(&response);
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.starts_with("spoticlean_login=;"))
    );
    let session_cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with("spoticlean_session="))
        .unwrap();
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/;"] {
        assert!(session_cookie.contains(attribute), "{attribute} missing");
    }
    assert!(!session_cookie.contains("access-token"));
    assert!(!session_cookie.contains("refresh-token"));

    let response = app
        .oneshot(request(
            Method::GET,
            "/api/v1/session",
            Some(&cookie_pair(session_cookie)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await,
        json!({"scopes": ["user-library-read", "user-library-modify"]})
    );
}

#[tokio::test]
async fn callback_with_another_state_never_exchanges_the_code() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let (login_cookie, _, _) = start_login(&app).await;
    let response = app
        .oneshot(request(
            Method::GET,
            "/auth/callback?code=the-code&state=forged",
            Some(&login_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/?authError=invalidState");
    assert!(
        !set_cookies(&response)
            .iter()
            .any(|cookie| cookie.starts_with("spoticlean_session="))
    );
}

#[tokio::test]
async fn callback_without_the_login_cookie_is_refused() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let (_, state, _) = start_login(&app).await;
    let response = app
        .oneshot(request(
            Method::GET,
            &format!("/auth/callback?code=the-code&state={state}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(location(&response), "/?authError=invalidState");
}

#[tokio::test]
async fn callback_reports_a_refused_consent() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let (login_cookie, state, _) = start_login(&app).await;
    let response = app
        .oneshot(request(
            Method::GET,
            &format!("/auth/callback?error=access_denied&state={state}"),
            Some(&login_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(location(&response), "/?authError=accessDenied");
}

#[tokio::test]
async fn callback_reports_a_rejected_code() {
    let mut accounts = MockAccounts::new();
    accounts.expect_exchange_code().returning(|_, _, _| {
        ready(Err(AccountsError::Rejected(
            "invalid_grant: bad code".to_owned(),
        )))
    });
    let app = app(MockLibrary::new(), accounts);
    let (login_cookie, state, _) = start_login(&app).await;
    let response = app
        .oneshot(request(
            Method::GET,
            &format!("/auth/callback?code=bad&state={state}"),
            Some(&login_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(location(&response), "/?authError=loginFailed");
}

async fn assert_problem(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let problem = body_json(response).await;
    assert_eq!(problem["status"], status.as_u16());
    assert_eq!(problem["type"], format!("urn:spoticlean:problem:{code}"));
}

#[tokio::test]
async fn protected_routes_need_a_session() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    for (method, uri) in [
        (Method::GET, "/api/v1/session"),
        (Method::GET, "/api/v1/scan"),
        (Method::POST, "/api/v1/removals"),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, uri, None))
            .await
            .unwrap();
        assert_problem(response, StatusCode::UNAUTHORIZED, "unauthenticated").await;
    }
}

#[tokio::test]
async fn forged_or_tampered_session_cookies_are_rejected() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let valid = valid_session_cookie();
    let mut tampered = valid.clone().into_bytes();
    let middle = tampered.len() / 2 + 10;
    tampered[middle] = if tampered[middle] == b'A' { b'B' } else { b'A' };
    let foreign = cookie_pair(
        CookieCodec::new(Key::from(&[8; 64]), true)
            .session(&session(3600, None))
            .to_str()
            .unwrap(),
    );
    for cookie in [
        "spoticlean_session=garbage".to_owned(),
        String::from_utf8(tampered).unwrap(),
        foreign,
        valid.replace("spoticlean_session=", "spoticlean_login="),
    ] {
        let response = app
            .clone()
            .oneshot(request(Method::GET, "/api/v1/session", Some(&cookie)))
            .await
            .unwrap();
        assert_problem(response, StatusCode::UNAUTHORIZED, "unauthenticated").await;
    }
}

#[tokio::test]
async fn an_expiring_session_is_renewed() {
    let mut accounts = MockAccounts::new();
    accounts
        .expect_refresh()
        .withf(|refresh| refresh.expose_secret() == "refresh-token")
        .times(1)
        .returning(|_| ready(Ok(grant("new-access", None))));
    let app = app(MockLibrary::new(), accounts);
    let response = app
        .oneshot(request(
            Method::GET,
            "/api/v1/session",
            Some(&session_cookie(60)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let renewed = set_cookies(&response);
    assert_eq!(renewed.len(), 1);
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, cookie_pair(&renewed[0]).parse().unwrap());
    let session = codec().read_session(&headers).unwrap();
    assert_eq!(session.access.expose_secret(), "new-access");
    assert_eq!(
        session.refresh.unwrap().expose_secret(),
        "refresh-token",
        "the refresh token Spotify did not rotate is kept"
    );
}

#[tokio::test]
async fn a_session_that_cannot_be_renewed_is_cleared() {
    let mut accounts = MockAccounts::new();
    accounts.expect_refresh().returning(|_| {
        ready(Err(AccountsError::Rejected(
            "invalid_grant: revoked".to_owned(),
        )))
    });
    let app = app(MockLibrary::new(), accounts);
    let response = app
        .oneshot(request(
            Method::GET,
            "/api/v1/scan",
            Some(&session_cookie(-10)),
        ))
        .await
        .unwrap();
    assert!(set_cookies(&response)[0].starts_with("spoticlean_session=;"));
    assert_problem(response, StatusCode::UNAUTHORIZED, "sessionExpired").await;
}

#[tokio::test]
async fn an_unreachable_accounts_service_is_a_bad_gateway() {
    let mut accounts = MockAccounts::new();
    accounts
        .expect_refresh()
        .returning(|_| ready(Err(AccountsError::Unavailable("timeout".to_owned()))));
    let app = app(MockLibrary::new(), accounts);
    let response = app
        .oneshot(request(
            Method::GET,
            "/api/v1/session",
            Some(&session_cookie(-10)),
        ))
        .await
        .unwrap();
    assert_eq!(set_cookies(&response), Vec::<String>::new());
    assert_problem(response, StatusCode::BAD_GATEWAY, "spotifyUnavailable").await;
}

#[tokio::test]
async fn logout_clears_the_session() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let response = app
        .oneshot(request(Method::DELETE, "/api/v1/session", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(set_cookies(&response)[0].starts_with("spoticlean_session=;"));
}

fn track(id: &str, title: &str, seconds: u64, release_date: &str) -> Track {
    Track {
        id: id.to_owned(),
        title: title.to_owned(),
        artists: vec!["The Beatles".to_owned()],
        album: format!("Album {id}"),
        release_date: Some(release_date.to_owned()),
        duration: Duration::from_secs(seconds),
        isrc: None,
        added_at: "2024-01-01T00:00:00Z".to_owned(),
    }
}

/// Splits an SSE body into `(event, data)` pairs, keep-alive comments left out.
fn sse_events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter_map(|block| {
            let mut event = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event: ") {
                    event = Some(value.to_owned());
                } else if let Some(value) = line.strip_prefix("data: ") {
                    data = Some(serde_json::from_str(value).unwrap());
                }
            }
            Some((event?, data?))
        })
        .collect()
}

async fn scan(library: MockLibrary) -> Vec<(String, Value)> {
    let response = app(library, MockAccounts::new())
        .oneshot(request(
            Method::GET,
            "/api/v1/scan",
            Some(&valid_session_cookie()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    sse_events(&body_text(response).await)
}

#[tokio::test]
async fn scan_streams_progress_then_the_groups() {
    let mut library = MockLibrary::new();
    library
        .expect_saved_tracks()
        .withf(|token, _, _| token.expose_secret() == "access-token")
        .returning(|_, offset, _| {
            let tracks = if offset == 0 {
                vec![track("a", "Let It Be", 243, "1970-05-08")]
            } else {
                vec![track("b", "Let It Be - Remastered 2009", 241, "2009-09-09")]
            };
            ready(Ok(SavedTracksPage { total: 60, tracks }))
        });
    let events = scan(library).await;
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["progress", "progress", "result"]);
    assert_eq!(events[0].1, json!({"fetched": 50, "total": 60}));
    assert_eq!(events[1].1, json!({"fetched": 60, "total": 60}));

    let result = &events[2].1;
    assert_eq!(result["scanned"], 2);
    let group = &result["groups"][0];
    assert_eq!(group["confidence"], "probable");
    assert_eq!(group["durationGapMs"], 2000);
    assert_eq!(group["key"].as_str().unwrap().len(), 24);
    assert_eq!(
        group["tracks"][1],
        json!({
            "id": "b",
            "title": "Let It Be - Remastered 2009",
            "artists": ["The Beatles"],
            "album": "Album b",
            "releaseDate": "2009-09-09",
            "durationMs": 241_000,
            "addedAt": "2024-01-01T00:00:00Z",
            "isrc": null,
            "versionLabel": "Remastered 2009",
        })
    );
    assert_eq!(group["tracks"][0]["versionLabel"], Value::Null);
}

#[tokio::test]
async fn scan_failure_ends_the_stream_with_an_error_event() {
    let mut library = MockLibrary::new();
    library
        .expect_saved_tracks()
        .returning(|_, _, _| ready(Err(ApiError::Unauthorized)));
    let events = scan(library).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "error");
    assert_eq!(events[0].1["status"], 401);
    assert_eq!(
        events[0].1["type"],
        "urn:spoticlean:problem:spotifyUnauthorized"
    );
}

fn track_ids(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("{index:0>22}")).collect()
}

async fn remove(library: MockLibrary, ids: &[String]) -> Response {
    app(library, MockAccounts::new())
        .oneshot(json_request(
            "/api/v1/removals",
            &valid_session_cookie(),
            &json!({ "trackIds": ids }),
        ))
        .await
        .unwrap()
}

#[tokio::test]
async fn removal_is_batched_by_forty() {
    let batches = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&batches);
    let mut library = MockLibrary::new();
    library
        .expect_remove_saved_tracks()
        .withf(|token, _| token.expose_secret() == "access-token")
        .times(3)
        .returning(move |_, ids| {
            seen.lock().unwrap().push(ids.len());
            ready(Ok(()))
        });
    let response = remove(library, &track_ids(85)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(*batches.lock().unwrap(), [40, 40, 5]);
    let body = body_json(response).await;
    assert_eq!(body["removed"], json!(track_ids(85)));
    assert_eq!(body["failed"], json!([]));
}

#[tokio::test]
async fn partial_removal_is_a_multi_status() {
    let calls = Mutex::new(0);
    let mut library = MockLibrary::new();
    library.expect_remove_saved_tracks().returning(move |_, _| {
        let mut calls = calls.lock().unwrap();
        *calls += 1;
        if *calls == 2 {
            ready(Err(ApiError::Forbidden(
                "Insufficient client scope".to_owned(),
            )))
        } else {
            ready(Ok(()))
        }
    });
    let response = remove(library, &track_ids(130)).await;
    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let body = body_json(response).await;
    assert_eq!(body["removed"], json!(track_ids(130)[..40]));
    let failed = body["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 3);
    assert_eq!(failed[0]["reason"], "spotifyForbidden");
    assert_eq!(failed[0]["trackIds"], json!(track_ids(130)[40..80]));
    assert!(
        failed[0]["detail"]
            .as_str()
            .unwrap()
            .contains("Insufficient client scope")
    );
    assert_eq!(failed[1]["reason"], "skipped");
    assert_eq!(failed[2]["trackIds"], json!(track_ids(130)[120..]));
}

#[tokio::test]
async fn removal_that_fails_entirely_is_a_problem() {
    let mut library = MockLibrary::new();
    library
        .expect_remove_saved_tracks()
        .times(1)
        .returning(|_, _| ready(Err(ApiError::Unauthorized)));
    let response = remove(library, &track_ids(50)).await;
    assert_problem(response, StatusCode::UNAUTHORIZED, "spotifyUnauthorized").await;
}

#[tokio::test]
async fn removal_rejects_invalid_requests() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let cookie = valid_session_cookie();
    for body in [
        json!({"trackIds": []}),
        json!({"trackIds": ["not-a-track-id"]}),
        json!({"trackIds": track_ids(10_001)}),
        json!({"tracks": track_ids(1)}),
    ] {
        let response = app
            .clone()
            .oneshot(json_request("/api/v1/removals", &cookie, &body))
            .await
            .unwrap();
        assert_problem(response, StatusCode::UNPROCESSABLE_ENTITY, "invalidRequest").await;
    }
    let mut no_content_type = json_request("/api/v1/removals", &cookie, &json!({}));
    no_content_type.headers_mut().remove(header::CONTENT_TYPE);
    let response = app.oneshot(no_content_type).await.unwrap();
    assert_problem(
        response,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "invalidRequest",
    )
    .await;
}

#[tokio::test]
async fn unknown_paths_are_problems_without_a_ui() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    for uri in ["/api/v1/nope", "/", "/review"] {
        let response = app
            .clone()
            .oneshot(request(Method::GET, uri, None))
            .await
            .unwrap();
        assert_problem(response, StatusCode::NOT_FOUND, "notFound").await;
    }
}

#[tokio::test]
async fn the_ui_is_served_with_a_fallback_to_index() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>ui</html>").unwrap();
    std::fs::write(dir.path().join("app.js"), "console.log(1)").unwrap();
    let app = app_with_static(MockLibrary::new(), MockAccounts::new(), Some(dir.path()));
    for (uri, body) in [
        ("/", "<html>ui</html>"),
        ("/review", "<html>ui</html>"),
        ("/app.js", "console.log(1)"),
    ] {
        let response = app
            .clone()
            .oneshot(request(Method::GET, uri, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert_eq!(body_text(response).await, body, "{uri}");
    }
    let response = app
        .oneshot(request(Method::GET, "/api/v1/nope", None))
        .await
        .unwrap();
    assert_problem(response, StatusCode::NOT_FOUND, "notFound").await;
}

#[tokio::test]
async fn an_empty_ui_directory_answers_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_static(MockLibrary::new(), MockAccounts::new(), Some(dir.path()));
    let response = app
        .oneshot(request(Method::GET, "/review", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn openapi_documents_every_operation() {
    let app = app(MockLibrary::new(), MockAccounts::new());
    let response = app
        .clone()
        .oneshot(request(Method::GET, "/openapi.json", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let document = body_json(response).await;
    let paths = document["paths"].as_object().unwrap();
    let mut documented: Vec<&str> = paths.keys().map(String::as_str).collect();
    documented.sort_unstable();
    assert_eq!(
        documented,
        [
            "/api/v1/removals",
            "/api/v1/scan",
            "/api/v1/session",
            "/auth/callback",
            "/auth/login"
        ]
    );
    for operation in paths
        .values()
        .flat_map(|path| path.as_object().unwrap().values())
    {
        assert!(
            operation["tags"]
                .as_array()
                .is_some_and(|tags| !tags.is_empty())
        );
    }
    assert!(
        paths["/api/v1/scan"]["get"]["responses"]["200"]["content"]["text/event-stream"]
            .is_object()
    );
    assert_eq!(
        document["components"]["securitySchemes"][SESSION_SCHEME]["in"],
        "cookie"
    );

    let response = app
        .clone()
        .oneshot(request(Method::GET, "/docs", None))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("/openapi.json"));
    let response = app
        .oneshot(request(Method::GET, "/healthz", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// The UI generates its API client from this copy, so it must follow every
/// change to the handlers. A stale copy is rewritten, and the test fails so
/// that the new one gets reviewed and committed.
#[tokio::test]
async fn the_committed_openapi_document_is_current() {
    let response = app(MockLibrary::new(), MockAccounts::new())
        .oneshot(request(Method::GET, "/openapi.json", None))
        .await
        .unwrap();
    let served = body_json(response).await;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/openapi.json");
    let committed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if committed.as_ref() != Some(&served) {
        let text = serde_json::to_string_pretty(&served).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        panic!(
            "{} was stale and has been rewritten: regenerate the UI's client with `npm run generate` in web/, then commit both",
            path.display()
        );
    }
}
