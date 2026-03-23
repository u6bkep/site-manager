use anyhow::{Context, Result, bail};

#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub data_dir: String,
    pub sites_dir: String,
    pub repos_dir: String,
    pub db_path: String,

    pub google_client_id: String,
    pub google_client_secret: String,
    pub allowed_domain: String,
    pub external_url: String,

    pub github_token: Option<String>,
    pub github_app_id: Option<u64>,
    pub github_app_private_key: Option<String>,
    pub github_app_installation_id: Option<u64>,
    pub github_webhook_secret: Option<String>,

    pub caddy_bin: String,
    pub caddy_root: String,
    pub caddy_tls: bool,
}

fn require_env(name: &str) -> Result<String> {
    let val = std::env::var(name)
        .with_context(|| format!("{} is required but not set", name))?;
    if val.trim().is_empty() {
        bail!("{} is set but empty", name);
    }
    Ok(val)
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = optional_env("DATA_DIR")
            .unwrap_or_else(|| "/var/lib/site-manager".into());
        let caddy_root = optional_env("CADDY_ROOT")
            .unwrap_or_else(|| "/etc/caddy".into());

        let external_url = optional_env("EXTERNAL_URL")
            .unwrap_or_else(|| "http://localhost:8080".into());
        let google_client_id = require_env("GOOGLE_CLIENT_ID")?;
        let allowed_domain = require_env("ALLOWED_DOMAIN")?;
        let bind_addr = optional_env("BIND_ADDR")
            .unwrap_or_else(|| "0.0.0.0:8080".into());

        let config = Self {
            bind_addr,
            sites_dir: optional_env("SITES_DIR")
                .unwrap_or_else(|| format!("{}/sites", &data_dir)),
            repos_dir: optional_env("REPOS_DIR")
                .unwrap_or_else(|| format!("{}/repos", &data_dir)),
            db_path: optional_env("DB_PATH")
                .unwrap_or_else(|| format!("{}/site-manager.db", &data_dir)),
            data_dir,

            google_client_id,
            google_client_secret: require_env("GOOGLE_CLIENT_SECRET")?,
            allowed_domain,
            external_url,

            github_token: optional_env("GITHUB_TOKEN"),
            github_app_id: optional_env("GITHUB_APP_ID")
                .and_then(|v| v.parse().ok()),
            github_app_private_key: optional_env("GITHUB_APP_PRIVATE_KEY"),
            github_app_installation_id: optional_env("GITHUB_APP_INSTALLATION_ID")
                .and_then(|v| v.parse().ok()),
            github_webhook_secret: optional_env("GITHUB_WEBHOOK_SECRET"),

            caddy_bin: optional_env("CADDY_BIN")
                .unwrap_or_else(|| "caddy".into()),
            caddy_tls: optional_env("CADDY_TLS")
                .map(|v| v.eq_ignore_ascii_case("on") || v == "true" || v == "1")
                .unwrap_or(false),
            caddy_root,
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        // EXTERNAL_URL must be a full URL
        if !self.external_url.starts_with("http://") && !self.external_url.starts_with("https://") {
            bail!(
                "EXTERNAL_URL must start with http:// or https:// (got '{}')\n  \
                 This is used to build OAuth redirect URIs. Example: https://sites.example.com",
                self.external_url
            );
        }
        if self.external_url.ends_with('/') {
            bail!(
                "EXTERNAL_URL must not have a trailing slash (got '{}')",
                self.external_url
            );
        }

        // GOOGLE_CLIENT_ID should look plausible
        if !self.google_client_id.contains('.') {
            bail!(
                "GOOGLE_CLIENT_ID doesn't look like a valid OAuth client ID (got '{}')\n  \
                 Expected format: <numbers>-<hash>.apps.googleusercontent.com\n  \
                 Get one at: https://console.cloud.google.com/apis/credentials",
                self.google_client_id
            );
        }

        // ALLOWED_DOMAIN should be a bare domain, not a URL or email
        if self.allowed_domain.contains("://") || self.allowed_domain.contains('@') || self.allowed_domain.contains('/') {
            bail!(
                "ALLOWED_DOMAIN should be a bare domain like 'example.com' (got '{}')",
                self.allowed_domain
            );
        }

        // BIND_ADDR should contain a port
        if !self.bind_addr.contains(':') {
            bail!(
                "BIND_ADDR should be host:port (got '{}')\n  Example: 0.0.0.0:8080",
                self.bind_addr
            );
        }

        // GitHub App config: all three must be set together
        let app_fields = [
            self.github_app_id.is_some(),
            self.github_app_private_key.is_some(),
            self.github_app_installation_id.is_some(),
        ];
        if app_fields.iter().any(|&v| v) && !app_fields.iter().all(|&v| v) {
            bail!(
                "GITHUB_APP_ID, GITHUB_APP_PRIVATE_KEY, and GITHUB_APP_INSTALLATION_ID must all be set together"
            );
        }

        // Warn about GitHub webhook without secret
        let has_github = self.github_token.is_some() || self.github_app_id.is_some();
        if has_github && self.github_webhook_secret.is_none() {
            tracing::warn!(
                "GITHUB_TOKEN is set but GITHUB_WEBHOOK_SECRET is not — \
                 webhook auto-deploy will reject all requests until a secret is configured"
            );
        }

        Ok(())
    }
}
