use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::Deserialize;

use crate::AppState;

const SESSION_COOKIE: &str = "session";
const OAUTH_STATE_COOKIE: &str = "oauth_state";
const REDIRECT_COOKIE: &str = "redirect_after";

pub struct AuthUser {
    pub email: String,
    pub name: String,
    pub picture_url: String,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    email: String,
    name: String,
    picture_url: String,
    expires_at: String,
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let is_api = parts.uri.path().starts_with("/api/");
        let reject = |msg: &str| -> Response {
            if is_api {
                (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": msg})),
                )
                    .into_response()
            } else {
                let path = parts
                    .uri
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/");
                Redirect::to(&format!("/login?redirect={}", urlencoding::encode(path)))
                    .into_response()
            }
        };

        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|c| c.value().to_string())
            .ok_or_else(|| reject("Not authenticated"))?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT email, name, picture_url, expires_at FROM sessions WHERE token = ?",
        )
        .bind(&token)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| reject("Session error"))?
        .ok_or_else(|| reject("Invalid session"))?;

        let expires =
            chrono::NaiveDateTime::parse_from_str(&row.expires_at, "%Y-%m-%d %H:%M:%S")
                .map_err(|_| reject("Session error"))?;
        if expires < chrono::Utc::now().naive_utc() {
            return Err(reject("Session expired"));
        }

        Ok(AuthUser {
            email: row.email,
            name: row.name,
            picture_url: row.picture_url,
        })
    }
}

// GET /login
pub async fn login_page() -> impl IntoResponse {
    crate::serve_embedded("login.html")
}

// GET /auth/google — start OAuth flow
#[derive(Deserialize)]
pub struct AuthStartParams {
    redirect: Option<String>,
}

pub async fn google_redirect(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuthStartParams>,
) -> impl IntoResponse {
    let csrf_token = generate_token();
    let redirect_after = params.redirect.unwrap_or_else(|| "/".into());

    let redirect_uri = format!("{}/auth/google/callback", state.config.external_url);
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={}&\
         redirect_uri={}&\
         response_type=code&\
         scope=openid%20email%20profile&\
         state={}&\
         hd={}",
        urlencoding::encode(&state.config.google_client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&csrf_token),
        urlencoding::encode(&state.config.allowed_domain),
    );

    let secure = state.config.external_url.starts_with("https://");

    let mut state_cookie = Cookie::new(OAUTH_STATE_COOKIE, csrf_token);
    state_cookie.set_http_only(true);
    state_cookie.set_path("/");
    state_cookie.set_same_site(SameSite::Lax);
    state_cookie.set_secure(secure);

    let mut redirect_cookie = Cookie::new(REDIRECT_COOKIE, redirect_after);
    redirect_cookie.set_http_only(true);
    redirect_cookie.set_path("/");
    redirect_cookie.set_same_site(SameSite::Lax);
    redirect_cookie.set_secure(secure);

    let jar = CookieJar::new().add(state_cookie).add(redirect_cookie);
    (jar, Redirect::to(&auth_url))
}

// GET /auth/google/callback
#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
    name: Option<String>,
    picture: Option<String>,
    hd: Option<String>,
}

pub async fn google_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackParams>,
    jar: CookieJar,
) -> Response {
    let redirect_after = jar
        .get(REDIRECT_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_else(|| "/".into());

    // Validate redirect target to prevent open redirects
    let redirect_after = if redirect_after.starts_with('/') && !redirect_after.starts_with("//") {
        redirect_after
    } else {
        "/".into()
    };

    // Remove OAuth cookies regardless of outcome
    let clean = |jar: CookieJar| -> CookieJar {
        let mut r1 = Cookie::new(OAUTH_STATE_COOKIE, "");
        r1.set_path("/");
        let mut r2 = Cookie::new(REDIRECT_COOKIE, "");
        r2.set_path("/");
        jar.remove(r1).remove(r2)
    };

    // Verify CSRF
    let valid_state = jar
        .get(OAUTH_STATE_COOKIE)
        .is_some_and(|c| c.value() == params.state);
    if !valid_state {
        return (
            clean(jar),
            Redirect::to("/login?error=Invalid+OAuth+state"),
        )
            .into_response();
    }

    // Exchange code for token
    let redirect_uri = format!("{}/auth/google/callback", state.config.external_url);
    let token_resp = state
        .http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", params.code.as_str()),
            ("client_id", state.config.google_client_id.as_str()),
            ("client_secret", state.config.google_client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await;

    let token_resp = match token_resp {
        Ok(r) => match r.json::<TokenResponse>().await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("token parse error: {}", e);
                return (
                    clean(jar),
                    Redirect::to("/login?error=Authentication+failed"),
                )
                    .into_response();
            }
        },
        Err(e) => {
            tracing::error!("token exchange error: {}", e);
            return (
                clean(jar),
                Redirect::to("/login?error=Authentication+failed"),
            )
                .into_response();
        }
    };

    // Get user info
    let user_info = match state
        .http_client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .header("Authorization", format!("Bearer {}", token_resp.access_token))
        .send()
        .await
    {
        Ok(r) => match r.json::<UserInfo>().await {
            Ok(u) => u,
            Err(e) => {
                tracing::error!("userinfo parse error: {}", e);
                return (
                    clean(jar),
                    Redirect::to("/login?error=Authentication+failed"),
                )
                    .into_response();
            }
        },
        Err(e) => {
            tracing::error!("userinfo error: {}", e);
            return (
                clean(jar),
                Redirect::to("/login?error=Authentication+failed"),
            )
                .into_response();
        }
    };

    // Check domain
    let domain = user_info.hd.as_deref().unwrap_or("");
    if domain != state.config.allowed_domain {
        return (
            clean(jar),
            Redirect::to("/login?error=Access+restricted+to+organization+members"),
        )
            .into_response();
    }

    // Create session
    let session_token = generate_token();
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    if let Err(e) = sqlx::query(
        "INSERT INTO sessions (token, email, name, picture_url, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&session_token)
    .bind(&user_info.email)
    .bind(user_info.name.as_deref().unwrap_or(""))
    .bind(user_info.picture.as_deref().unwrap_or(""))
    .bind(&expires_at)
    .execute(&state.db)
    .await
    {
        tracing::error!("session insert error: {}", e);
        return (
            clean(jar),
            Redirect::to("/login?error=Authentication+failed"),
        )
            .into_response();
    }

    let secure = state.config.external_url.starts_with("https://");

    let mut session_cookie = Cookie::new(SESSION_COOKIE, session_token);
    session_cookie.set_http_only(true);
    session_cookie.set_path("/");
    session_cookie.set_same_site(SameSite::Lax);
    session_cookie.set_secure(secure);

    (clean(jar).add(session_cookie), Redirect::to(&redirect_after)).into_response()
}

// GET /auth/verify — forward-auth endpoint for Caddy
pub async fn verify(State(state): State<Arc<AppState>>, jar: CookieJar) -> StatusCode {
    let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) else {
        return StatusCode::UNAUTHORIZED;
    };

    let Ok(Some(row)) = sqlx::query_as::<_, SessionRow>(
        "SELECT email, name, picture_url, expires_at FROM sessions WHERE token = ?",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await
    else {
        return StatusCode::UNAUTHORIZED;
    };

    let Ok(expires) = chrono::NaiveDateTime::parse_from_str(&row.expires_at, "%Y-%m-%d %H:%M:%S")
    else {
        return StatusCode::UNAUTHORIZED;
    };

    if expires < chrono::Utc::now().naive_utc() {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    }
}

// POST /auth/logout
pub async fn logout(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> impl IntoResponse {
    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) {
        let _ = sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(&token)
            .execute(&state.db)
            .await;
    }

    let mut removal = Cookie::new(SESSION_COOKIE, "");
    removal.set_path("/");

    (jar.remove(removal), Redirect::to("/login"))
}

// GET /api/me
pub async fn me(user: AuthUser) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "email": user.email,
        "name": user.name,
        "picture_url": user.picture_url,
    }))
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
