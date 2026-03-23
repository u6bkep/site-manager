use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::{AppState, auth::AuthUser, error::AppError};

#[derive(Serialize)]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub private: bool,
    pub default_branch: String,
}

#[derive(Deserialize)]
struct GithubRepo {
    full_name: String,
    name: String,
    owner: GithubOwner,
    private: bool,
    default_branch: String,
}

#[derive(Deserialize)]
struct GithubOwner {
    login: String,
}

#[derive(Serialize)]
pub struct Branch {
    pub name: String,
}

#[derive(Deserialize)]
struct GithubBranch {
    name: String,
}

// GET /api/github/repos
pub async fn list_repos(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Vec<Repo>>, AppError> {
    let provider = state.github_token_provider.as_ref()
        .ok_or_else(|| AppError::bad_request("GitHub not configured"))?;
    let token = provider.get_token().await
        .map_err(|e| AppError::bad_request(format!("GitHub auth failed: {}", e)))?;

    let is_app = state.config.github_app_id.is_some();

    let mut all_repos = Vec::new();
    let mut page = 1u32;

    loop {
        // GitHub App tokens use /installation/repositories; PATs use /user/repos
        let url = if is_app {
            format!(
                "https://api.github.com/installation/repositories?per_page=100&page={}",
                page
            )
        } else {
            format!(
                "https://api.github.com/user/repos?per_page=100&sort=updated&page={}",
                page
            )
        };

        if is_app {
            #[derive(Deserialize)]
            struct InstallationReposResponse {
                repositories: Vec<GithubRepo>,
            }

            let resp: InstallationReposResponse = state
                .http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .header("User-Agent", "site-manager")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?
                .json()
                .await?;

            let batch_len = resp.repositories.len();
            all_repos.extend(resp.repositories.into_iter().map(|r| Repo {
                full_name: r.full_name,
                name: r.name,
                owner: r.owner.login,
                private: r.private,
                default_branch: r.default_branch,
            }));

            if batch_len < 100 {
                break;
            }
        } else {
            let repos: Vec<GithubRepo> = state
                .http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .header("User-Agent", "site-manager")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?
                .json()
                .await?;

            let batch_len = repos.len();
            all_repos.extend(repos.into_iter().map(|r| Repo {
                full_name: r.full_name,
                name: r.name,
                owner: r.owner.login,
                private: r.private,
                default_branch: r.default_branch,
            }));

            if batch_len < 100 {
                break;
            }
        }

        page += 1;
    }

    Ok(Json(all_repos))
}

// GET /api/github/repos/{owner}/{repo}/branches
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<Vec<Branch>>, AppError> {
    let provider = state.github_token_provider.as_ref()
        .ok_or_else(|| AppError::bad_request("GitHub not configured"))?;
    let token = provider.get_token().await
        .map_err(|e| AppError::bad_request(format!("GitHub auth failed: {}", e)))?;

    let url = format!(
        "https://api.github.com/repos/{}/{}/branches?per_page=100",
        owner, repo
    );
    let branches: Vec<GithubBranch> = state
        .http_client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "site-manager")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .json()
        .await?;

    Ok(Json(
        branches.into_iter().map(|b| Branch { name: b.name }).collect(),
    ))
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
}

#[derive(Deserialize)]
struct GithubCommit {
    sha: String,
    commit: GithubCommitDetail,
}

#[derive(Deserialize)]
struct GithubCommitDetail {
    message: String,
}

// GET /api/github/repos/{owner}/{repo}/commits/{branch}
pub async fn latest_commit(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path((owner, repo, branch)): Path<(String, String, String)>,
) -> Result<Json<CommitInfo>, AppError> {
    let provider = state.github_token_provider.as_ref()
        .ok_or_else(|| AppError::bad_request("GitHub not configured"))?;
    let token = provider.get_token().await
        .map_err(|e| AppError::bad_request(format!("GitHub auth failed: {}", e)))?;

    let url = format!(
        "https://api.github.com/repos/{}/{}/commits/{}",
        owner, repo, branch
    );
    let commit: GithubCommit = state
        .http_client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "site-manager")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .json()
        .await?;

    Ok(Json(CommitInfo {
        sha: commit.sha,
        message: commit.commit.message.lines().next().unwrap_or("").to_string(),
    }))
}

// POST /api/github/webhook
#[derive(Deserialize)]
struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    repository: PushRepo,
    #[allow(dead_code)]
    after: String,
}

#[derive(Deserialize)]
struct PushRepo {
    full_name: String,
    clone_url: String,
}

pub async fn webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, AppError> {
    // Verify signature
    let secret = state
        .config
        .github_webhook_secret
        .as_ref()
        .ok_or_else(|| AppError::bad_request("Webhook verification not configured"))?;

    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::bad_request("Missing signature"))?;

    if !verify_signature(secret, &body, signature) {
        return Err(AppError::bad_request("Invalid signature"));
    }

    // Only handle push events
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event != "push" {
        return Ok(StatusCode::OK);
    }

    let push: PushEvent = serde_json::from_str(&body)
        .map_err(|_| AppError::bad_request("Invalid webhook payload"))?;

    // Extract branch name from ref (refs/heads/main -> main)
    let branch = push
        .git_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&push.git_ref);

    // Find sites tracking this repo and branch
    let repo_patterns = [
        push.repository.clone_url.clone(),
        format!("https://github.com/{}.git", push.repository.full_name),
        format!("https://github.com/{}", push.repository.full_name),
    ];

    #[derive(sqlx::FromRow)]
    struct SiteRow {
        slug: String,
        repo_url: Option<String>,
        branch: Option<String>,
        subdirectory: String,
    }

    let sites = sqlx::query_as::<_, SiteRow>(
        "SELECT slug, repo_url, branch, subdirectory FROM sites WHERE source_type = 'git'",
    )
    .fetch_all(&state.db)
    .await?;

    for site in sites {
        let site_branch = site.branch.as_deref().unwrap_or("main");
        let site_repo = site.repo_url.as_deref().unwrap_or("");

        let repo_matches = repo_patterns.iter().any(|p| p == site_repo);
        if repo_matches && site_branch == branch {
            tracing::info!(slug = %site.slug, branch, "webhook triggered deploy");

            let state_clone = state.clone();
            let slug = site.slug.clone();
            let repo_url = site_repo.to_string();
            let branch = branch.to_string();
            let subdir = site.subdirectory.clone();

            // Deploy in background
            tokio::spawn(async move {
                if let Err(_) = crate::sites::deploy_from_git(
                    &state_clone,
                    &slug,
                    &repo_url,
                    &branch,
                    &subdir,
                    "webhook",
                )
                .await
                {
                    tracing::error!(slug = %slug, "webhook deploy failed");
                }
            });
        }
    }

    Ok(StatusCode::OK)
}

fn verify_signature(secret: &str, body: &str, signature: &str) -> bool {
    let Some(sig_hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()
}
