mod auth;
mod caddy;
mod config;
mod db;
mod error;
mod github;
mod sites;

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/"]
struct Assets;

pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub config: config::Config,
    pub http_client: reqwest::Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = config::Config::from_env()?;

    // Ensure directories exist
    tokio::fs::create_dir_all(&config.data_dir).await?;
    tokio::fs::create_dir_all(&config.sites_dir).await?;
    tokio::fs::create_dir_all(&config.repos_dir).await?;

    let db = db::init(&config.db_path).await?;

    // Clean up expired sessions on startup
    sqlx::query("DELETE FROM sessions WHERE expires_at < datetime('now')")
        .execute(&db)
        .await?;

    let http_client = reqwest::Client::new();

    let state = Arc::new(AppState {
        db,
        config: config.clone(),
        http_client,
    });

    // Generate initial Caddyfile
    let caddyfile_content = caddy::generate_caddyfile(&state);
    let caddyfile_path = format!("{}/Caddyfile", config.caddy_root);
    tokio::fs::create_dir_all(&config.caddy_root).await?;
    tokio::fs::write(&caddyfile_path, &caddyfile_content).await?;
    tracing::info!("wrote initial Caddyfile to {}", caddyfile_path);

    let app = Router::new()
        // Health check
        .route("/healthz", get(health))
        // Public auth routes
        .route("/login", get(auth::login_page))
        .route("/auth/google", get(auth::google_redirect))
        .route("/auth/google/callback", get(auth::google_callback))
        .route("/auth/verify", get(auth::verify))
        .route("/auth/logout", post(auth::logout))
        // Authenticated pages
        .route("/", get(dashboard_page))
        .route("/sites/new", get(new_site_page))
        .route("/sites/{slug}", get(site_detail_page))
        // API
        .route("/api/me", get(auth::me))
        .route("/api/sites", get(sites::list).post(sites::create))
        .route(
            "/api/sites/{slug}",
            get(sites::get_site).delete(sites::delete_site).put(sites::update_site),
        )
        .route("/api/sites/{slug}/upload", post(sites::upload))
        .route("/api/sites/{slug}/deploy", post(sites::deploy))
        // GitHub
        .route("/api/github/repos", get(github::list_repos))
        .route(
            "/api/github/repos/{owner}/{repo}/branches",
            get(github::list_branches),
        )
        .route("/api/github/webhook", post(github::webhook))
        // Site preview (dev mode — serves sites directly with auth)
        .route("/s/{*path}", get(serve_site))
        // Static assets
        .route("/assets/{*path}", get(serve_asset))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("listening on {}", config.bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

// Page handlers — serve embedded HTML after auth check
async fn dashboard_page(_user: auth::AuthUser) -> Response {
    serve_embedded("index.html")
}

async fn new_site_page(_user: auth::AuthUser) -> Response {
    serve_embedded("new.html")
}

async fn site_detail_page(_user: auth::AuthUser, Path(_slug): Path<String>) -> Response {
    serve_embedded("site.html")
}

// Serve embedded static assets (CSS, JS)
async fn serve_asset(Path(path): Path<String>) -> Response {
    serve_embedded(&path)
}

fn serve_embedded(path: &str) -> Response {
    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// Dev-mode site serving with auth
async fn serve_site(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Path(path): Path<String>,
) -> Response {
    // Check auth via session cookie
    let authenticated = if let Some(token) = jar.get("session").map(|c| c.value().to_string()) {
        sqlx::query_scalar::<_, String>(
            "SELECT email FROM sessions WHERE token = ? AND expires_at > datetime('now')",
        )
        .bind(&token)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .is_some()
    } else {
        false
    };

    if !authenticated {
        let redirect = format!("/login?redirect=/s/{}", urlencoding::encode(&path));
        return axum::response::Redirect::to(&redirect).into_response();
    }

    // Parse path: slug/file_path
    let (slug, file_path) = match path.split_once('/') {
        Some((s, f)) => (s, f),
        None => (path.as_str(), "index.html"),
    };

    let mut full_path = std::path::PathBuf::from(&state.config.sites_dir)
        .join(slug)
        .join(file_path);

    // Reject path traversal attempts
    if std::path::Path::new(file_path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Directory → index.html
    if full_path.is_dir() {
        full_path = full_path.join("index.html");
    }

    match tokio::fs::read(&full_path).await {
        Ok(content) => {
            let mime = mime_guess::from_path(&full_path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
