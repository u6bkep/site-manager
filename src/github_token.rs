use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::sync::RwLock;

use crate::config::Config;

/// Provides GitHub API tokens. Supports either a static PAT (GITHUB_TOKEN)
/// or GitHub App authentication with automatic token refresh.
pub struct GitHubTokenProvider {
    config: Config,
    http_client: reqwest::Client,
    /// Cached installation token + expiry (GitHub App mode only)
    cached_token: RwLock<Option<CachedToken>>,
}

struct CachedToken {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl GitHubTokenProvider {
    pub fn new(config: &Config, http_client: &reqwest::Client) -> Option<Arc<Self>> {
        if config.github_token.is_some() || config.github_app_id.is_some() {
            Some(Arc::new(Self {
                config: config.clone(),
                http_client: http_client.clone(),
                cached_token: RwLock::new(None),
            }))
        } else {
            None
        }
    }

    /// Get a valid GitHub token, refreshing if necessary.
    pub async fn get_token(&self) -> Result<String> {
        // Static PAT mode
        if let Some(ref token) = self.config.github_token {
            return Ok(token.clone());
        }

        // GitHub App mode — check cache first
        {
            let cached = self.cached_token.read().await;
            if let Some(ref ct) = *cached {
                // Refresh 5 minutes before expiry
                if ct.expires_at > chrono::Utc::now() + chrono::Duration::minutes(5) {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Need to refresh
        let token = self.fetch_installation_token().await?;
        Ok(token)
    }

    async fn fetch_installation_token(&self) -> Result<String> {
        let app_id = self.config.github_app_id
            .context("GITHUB_APP_ID not configured")?;
        let private_key = self.config.github_app_private_key.as_ref()
            .context("GITHUB_APP_PRIVATE_KEY not configured")?;
        let installation_id = self.config.github_app_installation_id
            .context("GITHUB_APP_INSTALLATION_ID not configured")?;

        // Create JWT for GitHub App authentication
        let jwt = create_app_jwt(app_id, private_key)?;

        // Exchange JWT for an installation token
        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            installation_id
        );

        let resp = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("User-Agent", "site-manager")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .context("failed to request installation token")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("GitHub App token request failed ({}): {}", status, body);
        }

        #[derive(serde::Deserialize)]
        struct TokenResponse {
            token: String,
            expires_at: String,
        }

        let token_resp: TokenResponse = resp.json().await
            .context("failed to parse installation token response")?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&token_resp.expires_at)
            .context("failed to parse token expiry")?
            .with_timezone(&chrono::Utc);

        let token = token_resp.token.clone();

        // Cache the token
        let mut cached = self.cached_token.write().await;
        *cached = Some(CachedToken {
            token: token_resp.token,
            expires_at,
        });

        tracing::debug!("refreshed GitHub App installation token, expires at {}", expires_at);
        Ok(token)
    }
}

/// Create a JWT signed with the GitHub App's private key (RS256).
fn create_app_jwt(app_id: u64, private_key_pem: &str) -> Result<String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header};

    let now = chrono::Utc::now();

    #[derive(serde::Serialize)]
    struct Claims {
        iat: i64,
        exp: i64,
        iss: String,
    }

    let claims = Claims {
        iat: (now - chrono::Duration::seconds(60)).timestamp(),
        exp: (now + chrono::Duration::minutes(9)).timestamp(), // Max 10 min, use 9 for safety
        iss: app_id.to_string(),
    };

    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .context("invalid GitHub App private key (expected PEM-encoded RSA key)")?;

    let header = Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &key)
        .context("failed to sign JWT for GitHub App")
}
