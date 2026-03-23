# Site Manager

A self-hosted platform for deploying and serving static websites behind Google Workspace authentication. Designed for teams where non-technical members need to publish simple static sites (HTML/JS tools, dashboards, internal pages) without touching git or infrastructure.

## Features

- **Google Workspace auth** — login restricted to your organization's domain. One login protects all sites.
- **Upload or Git deploy** — drag-and-drop a zip file, or connect a GitHub repo and track a branch.
- **Auto-deploy on push** — GitHub webhooks trigger redeployment when the tracked branch is updated.
- **Management UI** — create, preview (via iframe), redeploy, and delete sites from the browser.
- **Custom domain support** — sites are served at `/s/{slug}/` by default, with optional per-site custom domains.
- **Caddy integration** — generates Caddyfile with forward-auth and file serving. In dev mode, the app serves everything directly.

## Quick Start

### Prerequisites

- Rust 1.75+
- A Google Cloud project with OAuth credentials ([setup guide](#google-oauth-setup))
- Optional: a GitHub Personal Access Token for private repo support

### Run locally

```sh
GOOGLE_CLIENT_ID="your-client-id.apps.googleusercontent.com" \
GOOGLE_CLIENT_SECRET="your-secret" \
ALLOWED_DOMAIN="yourdomain.com" \
EXTERNAL_URL="http://localhost:8080" \
cargo run
```

Open `http://localhost:8080`, sign in with your Google Workspace account, and create a site.

### Run with Docker

```sh
cp .env.example .env
# Edit .env with your credentials
docker compose up --build
```

## Configuration

All configuration is via environment variables.

| Variable | Required | Default | Description |
|---|---|---|---|
| `GOOGLE_CLIENT_ID` | yes | | OAuth client ID from Google Cloud Console |
| `GOOGLE_CLIENT_SECRET` | yes | | OAuth client secret |
| `ALLOWED_DOMAIN` | yes | | Google Workspace domain (e.g. `mycompany.com`) |
| `EXTERNAL_URL` | no | `http://localhost:8080` | Public URL of the app (used for OAuth redirects) |
| `BIND_ADDR` | no | `0.0.0.0:8080` | Listen address |
| `DATA_DIR` | no | `/var/lib/site-manager` | Root directory for all data |
| `SITES_DIR` | no | `{DATA_DIR}/sites` | Where deployed site files are stored |
| `REPOS_DIR` | no | `{DATA_DIR}/repos` | Where git clones are kept |
| `DB_PATH` | no | `{DATA_DIR}/site-manager.db` | SQLite database path |
| `GITHUB_TOKEN` | no | | GitHub PAT for private repo access (simple setup) |
| `GITHUB_APP_ID` | no | | GitHub App ID (org-level auth, recommended) |
| `GITHUB_APP_PRIVATE_KEY` | no | | GitHub App PEM private key |
| `GITHUB_APP_INSTALLATION_ID` | no | | GitHub App installation ID |
| `GITHUB_WEBHOOK_SECRET` | no | | Secret for verifying GitHub webhook signatures |
| `CADDY_TLS` | no | `off` | `on` for Caddy to manage TLS via ACME; `off` when behind a TLS-terminating reverse proxy |
| `CADDY_BIN` | no | `caddy` | Path to Caddy binary (for production reloads) |
| `CADDY_ROOT` | no | `/etc/caddy` | Caddy configuration directory |
| `RUST_LOG` | no | `info` | Log level (`debug`, `info`, `warn`, `error`) |

## Google OAuth Setup

1. Go to [Google Cloud Console](https://console.cloud.google.com/) → APIs & Services → OAuth consent screen.
2. Set user type to **Internal** (restricts to your Workspace domain).
3. Add scopes: `email`, `profile`, `openid`.
4. Go to Credentials → Create Credentials → OAuth client ID.
5. Application type: **Web application**.
6. Add authorized redirect URI: `{EXTERNAL_URL}/auth/google/callback`.
7. Copy the client ID and secret into your environment.

## GitHub Integration

Two authentication methods are supported. GitHub App (Option B) is recommended for organizations.

### Option A: Personal Access Token

1. Create a [Personal Access Token](https://github.com/settings/tokens) with `repo` scope (classic) or fine-grained with repository read access.
2. Set `GITHUB_TOKEN` in your environment.

### Option B: GitHub App (recommended)

1. Go to your org's Settings → Developer settings → GitHub Apps → **New GitHub App**.
2. Set a name (e.g. "Site Manager"), homepage URL to your `EXTERNAL_URL`.
3. Uncheck **Webhook → Active** (the app handles webhooks separately).
4. Permissions: **Repository → Contents → Read-only**.
5. Where can this app be installed? → **Only on this account**.
6. Create the app. Note the **App ID** from the app's settings page.
7. Generate a **private key** (downloads a `.pem` file).
8. Install the app on your org (Settings → Developer settings → GitHub Apps → Install). Note the **Installation ID** from the URL (`/installations/<id>`).
9. Set environment variables:
   ```sh
   GITHUB_APP_ID=123456
   GITHUB_APP_PRIVATE_KEY="$(cat path/to/private-key.pem)"
   GITHUB_APP_INSTALLATION_ID=12345678
   ```

### Webhooks (auto-deploy on push)

For either option, to enable auto-deploy set `GITHUB_WEBHOOK_SECRET` to a random string, then configure a webhook on each repo (or at the org level):
   - Payload URL: `{EXTERNAL_URL}/api/github/webhook`
   - Content type: `application/json`
   - Secret: same value as `GITHUB_WEBHOOK_SECRET`
   - Events: just **Pushes**

## API

All API endpoints require an authenticated session cookie.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/me` | Current user info |
| `GET` | `/api/sites` | List all sites |
| `POST` | `/api/sites` | Create a site (JSON body) |
| `GET` | `/api/sites/{slug}` | Get site details |
| `DELETE` | `/api/sites/{slug}` | Delete a site and its files |
| `POST` | `/api/sites/{slug}/upload` | Upload a zip (multipart, upload-type sites) |
| `POST` | `/api/sites/{slug}/deploy` | Trigger redeploy (git-type sites) |
| `GET` | `/api/github/repos` | List repos accessible to the configured token |
| `GET` | `/api/github/repos/{owner}/{repo}/branches` | List branches |
| `POST` | `/api/github/webhook` | GitHub webhook receiver |

### Create site (git)

```sh
curl -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"name":"my-app","source_type":"git","repo_url":"https://github.com/org/repo","branch":"main","subdirectory":"dist"}' \
  http://localhost:8080/api/sites
```

### Create site (upload)

```sh
# Create the site
curl -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"name":"my-app","source_type":"upload"}' \
  http://localhost:8080/api/sites

# Upload content
curl -b cookies.txt -F 'file=@site.zip' \
  http://localhost:8080/api/sites/my-app/upload
```

## Production Deployment

In production, Caddy sits in front and handles TLS, static file serving, and forward-auth:

```
Internet → Caddy (:443) → forward_auth → App (:8080)
                         → file_server (static sites)
```

The app generates a Caddyfile with the right routing and can reload Caddy when sites change. Set `EXTERNAL_URL` to your public `https://` URL and configure DNS to point at your server. Caddy handles Let's Encrypt certificates automatically.

## Project Structure

```
src/
├── main.rs        Entry point, router, embedded asset serving
├── config.rs      Environment variable configuration
├── db.rs          SQLite pool and migrations
├── auth.rs        Google OAuth, sessions, forward-auth
├── sites.rs       Site CRUD, upload, git clone/pull
├── github.rs      GitHub API integration, webhook handler
├── caddy.rs       Caddyfile generation and reload
└── error.rs       Error type for API responses
web/
├── login.html     Login page
├── index.html     Dashboard
├── new.html       Create site form
├── site.html      Site detail with iframe preview
├── style.css      Styles
└── app.js         Shared JS utilities
migrations/
└── 20240101000000_initial.sql
```
