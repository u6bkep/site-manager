# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A self-hosted platform for deploying and serving static websites behind Google Workspace authentication. Non-technical users can publish static sites via zip upload or GitHub repo connection, with auto-deploy on push via webhooks.

## Build & Run

```sh
# Run locally (requires Rust 1.75+, env vars for Google OAuth)
GOOGLE_CLIENT_ID="..." GOOGLE_CLIENT_SECRET="..." ALLOWED_DOMAIN="example.com" cargo run

# Build release
cargo build --release

# Run with Docker
cp .env.example .env  # then edit
docker compose up --build
```

There are no tests or linting configured in this project currently.

## Architecture

**Rust backend (Axum)** with SQLite (via sqlx) and embedded frontend assets (via rust-embed).

### Key design decisions:
- **Two deployment modes**: In dev, the Axum server serves everything directly (sites at `/s/{slug}/`). In production, Caddy sits in front handling TLS, static file serving with forward-auth, and the app generates/reloads Caddyfiles.
- **Auth flow**: Google OAuth with `hd` domain restriction. Sessions stored in SQLite with cookie-based auth. `AuthUser` is an Axum extractor — including it in a handler's signature enforces authentication.
- **Forward-auth**: `/auth/verify` returns 200/401 for Caddy's `forward_auth` directive. In dev mode, `serve_site` in `main.rs` duplicates this check inline.
- **Git operations use libgit2** (via `git2` crate) rather than shelling out to git. Auth for private GitHub repos uses `x-access-token` credential with the configured PAT.
- **Web frontend**: Vanilla HTML/JS/CSS in `web/`, embedded into the binary at compile time. `app.js` provides shared utilities (`api()`, `apiJson()`, `toast()`, etc.) used by all pages.

### Module responsibilities:
- `main.rs` — Router setup, embedded asset serving, dev-mode site serving with auth
- `auth.rs` — Google OAuth flow, session management, `AuthUser` extractor, forward-auth endpoint
- `sites.rs` — Site CRUD, zip upload with extraction hardening, git clone/pull via libgit2, deployment records
- `github.rs` — GitHub API (list repos/branches), webhook receiver with HMAC-SHA256 signature verification
- `caddy.rs` — Caddyfile generation from app state, Caddy reload via subprocess
- `config.rs` — All config from env vars, validation on startup
- `error.rs` — `AppError` type that maps to JSON error responses (hides internal errors for 500s)

### Database:
SQLite with sqlx compile-time migrations from `migrations/`. Three tables: `sessions`, `sites`, `deployments`. The `sites` table has a `source_type` field (`upload` or `git`) that determines which operations are valid.

## Configuration

All via environment variables. Required: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `ALLOWED_DOMAIN`. See README.md for the full table.
