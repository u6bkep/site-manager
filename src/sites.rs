use std::{path::{Component, PathBuf}, sync::Arc};

use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{AppState, auth::AuthUser, error::AppError};

#[derive(Serialize, sqlx::FromRow)]
pub struct Site {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub source_type: String,
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub subdirectory: String,
    pub public: bool,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_deployed_at: Option<String>,
    pub last_commit_sha: Option<String>,
    pub last_commit_message: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateSite {
    pub name: String,
    pub source_type: String,
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub subdirectory: Option<String>,
    pub public: Option<bool>,
}

// GET /api/sites
pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Vec<Site>>, AppError> {
    let sites = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites ORDER BY updated_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(sites))
}

// POST /api/sites
pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<CreateSite>,
) -> Result<(StatusCode, Json<Site>), AppError> {
    let slug = slugify(&body.name);
    if slug.is_empty() {
        return Err(AppError::bad_request("Invalid site name"));
    }

    // Check for duplicate slug
    let existing = sqlx::query("SELECT id FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?;
    if existing.is_some() {
        return Err(AppError::bad_request(format!(
            "A site with slug '{}' already exists",
            slug
        )));
    }

    // Validate repo_url uses HTTPS
    if let Some(ref repo_url) = body.repo_url {
        if !repo_url.starts_with("https://") {
            return Err(AppError::bad_request(
                "Repository URL must use HTTPS (start with https://)",
            ));
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let subdirectory = body.subdirectory.unwrap_or_default();

    // Validate subdirectory does not contain parent directory traversal
    if !subdirectory.is_empty() {
        let subdir_path = std::path::Path::new(&subdirectory);
        for component in subdir_path.components() {
            if matches!(component, Component::ParentDir) {
                return Err(AppError::bad_request(
                    "Subdirectory must not contain '..' path segments",
                ));
            }
        }
    }

    let public = body.public.unwrap_or(false);

    sqlx::query(
        "INSERT INTO sites (id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&slug)
    .bind(&body.name)
    .bind(&body.source_type)
    .bind(&body.repo_url)
    .bind(&body.branch)
    .bind(&subdirectory)
    .bind(public)
    .bind(&user.email)
    .execute(&state.db)
    .await?;

    // Create site directory
    let site_dir = PathBuf::from(&state.config.sites_dir).join(&slug);
    tokio::fs::create_dir_all(&site_dir).await?;

    // If git source, clone the repo
    if body.source_type == "git" {
        if let Some(ref repo_url) = body.repo_url {
            let branch = body.branch.clone().unwrap_or_else(|| "main".into());
            if let Err(e) = deploy_from_git(&state, &slug, repo_url, &branch, &subdirectory, &user.email)
                .await
            {
                // Clean up the site record and directories on failed initial deploy
                let _ = sqlx::query("DELETE FROM sites WHERE id = ?")
                    .bind(&id)
                    .execute(&state.db)
                    .await;
                let _ = tokio::fs::remove_dir_all(PathBuf::from(&state.config.sites_dir).join(&slug)).await;
                let _ = tokio::fs::remove_dir_all(PathBuf::from(&state.config.repos_dir).join(&slug)).await;
                return Err(e);
            }
        }
    }

    if let Err(e) = crate::caddy::reload_caddy(&state).await {
        tracing::warn!("caddy reload failed: {}", e);
    }

    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(site)))
}

// GET /api/sites/{slug}
pub async fn get_site(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Json<Site>, AppError> {
    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Site not found"))?;
    Ok(Json(site))
}

#[derive(Deserialize)]
pub struct UpdateSite {
    pub name: Option<String>,
    pub branch: Option<String>,
    pub subdirectory: Option<String>,
    pub public: Option<bool>,
}

// PUT /api/sites/{slug}
pub async fn update_site(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(slug): Path<String>,
    Json(body): Json<UpdateSite>,
) -> Result<Json<Site>, AppError> {
    // Verify site exists
    let _existing = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Site not found"))?;

    // Validate subdirectory if provided
    if let Some(ref subdirectory) = body.subdirectory {
        if !subdirectory.is_empty() {
            let subdir_path = std::path::Path::new(subdirectory);
            for component in subdir_path.components() {
                if matches!(component, Component::ParentDir) {
                    return Err(AppError::bad_request(
                        "Subdirectory must not contain '..' path segments",
                    ));
                }
            }
        }
    }

    if let Some(ref name) = body.name {
        sqlx::query("UPDATE sites SET name = ?, updated_at = datetime('now') WHERE slug = ?")
            .bind(name)
            .bind(&slug)
            .execute(&state.db)
            .await?;
    }

    if let Some(ref branch) = body.branch {
        sqlx::query("UPDATE sites SET branch = ?, updated_at = datetime('now') WHERE slug = ?")
            .bind(branch)
            .bind(&slug)
            .execute(&state.db)
            .await?;
    }

    if let Some(ref subdirectory) = body.subdirectory {
        sqlx::query("UPDATE sites SET subdirectory = ?, updated_at = datetime('now') WHERE slug = ?")
            .bind(subdirectory)
            .bind(&slug)
            .execute(&state.db)
            .await?;
    }

    if let Some(public) = body.public {
        sqlx::query("UPDATE sites SET public = ?, updated_at = datetime('now') WHERE slug = ?")
            .bind(public)
            .bind(&slug)
            .execute(&state.db)
            .await?;
    }

    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_one(&state.db)
        .await?;

    Ok(Json(site))
}

// DELETE /api/sites/{slug}
pub async fn delete_site(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query("DELETE FROM sites WHERE slug = ?")
        .bind(&slug)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Site not found"));
    }

    // Clean up files
    let site_dir = PathBuf::from(&state.config.sites_dir).join(&slug);
    if site_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&site_dir).await;
    }
    let repo_dir = PathBuf::from(&state.config.repos_dir).join(&slug);
    if repo_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&repo_dir).await;
    }

    if let Err(e) = crate::caddy::reload_caddy(&state).await {
        tracing::warn!("caddy reload failed: {}", e);
    }

    Ok(StatusCode::NO_CONTENT)
}

// POST /api/sites/{slug}/upload
pub async fn upload(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<Site>, AppError> {
    // Verify site exists and is upload type
    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Site not found"))?;

    if site.source_type != "upload" {
        return Err(AppError::bad_request(
            "Can only upload to upload-type sites",
        ));
    }

    // Read the uploaded file
    let mut file_data = None;
    while let Some(field) = multipart.next_field().await? {
        if field.name() == Some("file") {
            file_data = Some(field.bytes().await?);
            break;
        }
    }
    let file_data = file_data.ok_or_else(|| AppError::bad_request("No file uploaded"))?;

    // Extract zip to site directory
    let site_dir = PathBuf::from(&state.config.sites_dir).join(&slug);

    // Clear existing content
    if site_dir.exists() {
        tokio::fs::remove_dir_all(&site_dir).await?;
    }
    tokio::fs::create_dir_all(&site_dir).await?;

    // Extract zip with hardening
    let site_dir_clone = site_dir.clone();
    let data = file_data.to_vec();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        const MAX_EXTRACT_SIZE: u64 = 500 * 1024 * 1024; // 500MB

        let cursor = std::io::Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)?;
        let canonical_target = std::fs::canonicalize(&site_dir_clone)?;
        let mut cumulative_size: u64 = 0;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Some(entry_path) = entry.enclosed_name() else {
                anyhow::bail!("Zip entry has invalid path");
            };

            // Check for .. components
            for component in entry_path.components() {
                if matches!(component, Component::ParentDir) {
                    anyhow::bail!(
                        "Zip entry contains '..' path component: {}",
                        entry_path.display()
                    );
                }
            }

            let out_path = site_dir_clone.join(&entry_path);

            // Verify the resolved path stays within the target directory
            // For directories, create them and check; for files, check the parent
            if entry.is_dir() {
                std::fs::create_dir_all(&out_path)?;
                let canonical = std::fs::canonicalize(&out_path)?;
                if !canonical.starts_with(&canonical_target) {
                    anyhow::bail!(
                        "Zip entry escapes target directory: {}",
                        entry_path.display()
                    );
                }
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                    let canonical_parent = std::fs::canonicalize(parent)?;
                    if !canonical_parent.starts_with(&canonical_target) {
                        anyhow::bail!(
                            "Zip entry escapes target directory: {}",
                            entry_path.display()
                        );
                    }
                }

                // Track cumulative size
                cumulative_size += entry.size();
                if cumulative_size > MAX_EXTRACT_SIZE {
                    anyhow::bail!(
                        "Zip archive exceeds maximum extraction size of 500MB"
                    );
                }

                let mut outfile = std::fs::File::create(&out_path)?;
                std::io::copy(&mut entry, &mut outfile)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::bad_request(format!("Zip extraction failed: {}", e)))?
    .map_err(|e| AppError::bad_request(format!("Zip extraction failed: {}", e)))?;

    // Record deployment
    record_deployment(&state.db, &site.id, &user.email, None, "success", None).await?;

    // Update last_deployed_at
    sqlx::query("UPDATE sites SET last_deployed_at = datetime('now'), updated_at = datetime('now') WHERE slug = ?")
        .bind(&slug)
        .execute(&state.db)
        .await?;

    if let Err(e) = crate::caddy::reload_caddy(&state).await {
        tracing::warn!("caddy reload failed: {}", e);
    }

    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_one(&state.db)
        .await?;

    Ok(Json(site))
}

// POST /api/sites/{slug}/deploy
pub async fn deploy(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Json<Site>, AppError> {
    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Site not found"))?;

    if site.source_type != "git" {
        return Err(AppError::bad_request("Can only deploy git-type sites"));
    }

    let repo_url = site
        .repo_url
        .as_ref()
        .ok_or_else(|| AppError::bad_request("No repo URL configured"))?;
    let branch = site
        .branch
        .as_deref()
        .unwrap_or("main");

    deploy_from_git(
        &state,
        &slug,
        repo_url,
        branch,
        &site.subdirectory,
        &user.email,
    )
    .await?;

    if let Err(e) = crate::caddy::reload_caddy(&state).await {
        tracing::warn!("caddy reload failed: {}", e);
    }

    let site = sqlx::query_as::<_, Site>("SELECT id, slug, name, source_type, repo_url, branch, subdirectory, public, created_by, created_at, updated_at, last_deployed_at, last_commit_sha, last_commit_message FROM sites WHERE slug = ?")
        .bind(&slug)
        .fetch_one(&state.db)
        .await?;

    Ok(Json(site))
}

pub async fn deploy_from_git(
    state: &AppState,
    slug: &str,
    repo_url: &str,
    branch: &str,
    subdirectory: &str,
    deployed_by: &str,
) -> Result<(), AppError> {
    let repo_dir = PathBuf::from(&state.config.repos_dir).join(slug);
    let site_dir = PathBuf::from(&state.config.sites_dir).join(slug);

    tokio::fs::create_dir_all(&repo_dir).await?;
    tokio::fs::create_dir_all(&site_dir).await?;

    // Validate repo_url uses HTTPS
    if !repo_url.starts_with("https://") {
        return Err(AppError::bad_request(
            "Repository URL must use HTTPS (start with https://)",
        ));
    }

    let is_github = repo_url.contains("github.com");

    let repo_url = repo_url.to_string();
    let branch = branch.to_string();
    let subdir = subdirectory.to_string();
    let repo_dir_clone = repo_dir.clone();
    let site_dir_clone = site_dir.clone();
    let github_token = if is_github {
        if let Some(ref provider) = state.github_token_provider {
            Some(provider.get_token().await.map_err(|e| {
                AppError::bad_request(format!("GitHub auth failed: {}", e))
            })?)
        } else {
            None
        }
    } else {
        None
    };

    let (commit_sha, commit_message) = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String)> {
        let clone_fresh = |dir: &std::path::Path| -> anyhow::Result<git2::Repository> {
            let mut builder = git2::build::RepoBuilder::new();
            let mut fetch_opts = git2::FetchOptions::new();
            if let Some(ref token) = github_token {
                let mut callbacks = git2::RemoteCallbacks::new();
                let token = token.clone();
                callbacks.credentials(move |_url, _username, _allowed| {
                    git2::Cred::userpass_plaintext("x-access-token", &token)
                });
                fetch_opts.remote_callbacks(callbacks);
            }
            fetch_opts.depth(1);
            builder.fetch_options(fetch_opts);
            builder.branch(&branch);
            Ok(builder.clone(&repo_url, dir)?)
        };

        // Clone or pull
        let repo = if repo_dir_clone.join(".git").exists() {
            // Try fetch-and-reset on existing repo; fall back to fresh clone
            // if it fails (e.g. branch switch on a shallow clone)
            let fetch_result: anyhow::Result<git2::Repository> = (|| {
                let repo = git2::Repository::open(&repo_dir_clone)?;

                // Check if we already track this branch
                let remote_ref = format!("refs/remotes/origin/{}", branch);
                let has_branch = repo.find_reference(&remote_ref).is_ok();

                let mut remote = repo.find_remote("origin")?;
                let mut fetch_opts = git2::FetchOptions::new();
                if let Some(ref token) = github_token {
                    let mut callbacks = git2::RemoteCallbacks::new();
                    let token = token.clone();
                    callbacks.credentials(move |_url, _username, _allowed| {
                        git2::Cred::userpass_plaintext("x-access-token", &token)
                    });
                    fetch_opts.remote_callbacks(callbacks);
                }

                if has_branch {
                    // Branch already tracked — shallow fetch update
                    fetch_opts.depth(1);
                    remote.fetch(&[&branch], Some(&mut fetch_opts), None)?;
                } else {
                    // New branch — explicit refspec to create tracking ref
                    remote.fetch(
                        &[&format!("+refs/heads/{}:{}", branch, remote_ref)],
                        Some(&mut fetch_opts),
                        None,
                    )?;
                }
                drop(remote);

                {
                    // Prefer remote tracking ref, fall back to FETCH_HEAD
                    let reference = repo.find_reference(&remote_ref)
                        .or_else(|_| repo.find_reference("FETCH_HEAD"))?;
                    let commit = reference.peel_to_commit()?;
                    repo.reset(commit.as_object(), git2::ResetType::Hard, None)?;
                }
                Ok(repo)
            })();

            match fetch_result {
                Ok(repo) => repo,
                Err(e) => {
                    tracing::warn!("fetch failed, re-cloning: {}", e);
                    std::fs::remove_dir_all(&repo_dir_clone)?;
                    clone_fresh(&repo_dir_clone)?
                }
            }
        } else {
            clone_fresh(&repo_dir_clone)?
        };

        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        let sha = commit.id().to_string();
        let message = commit.summary().unwrap_or("").to_string();

        // Copy files from repo (optionally from subdirectory) to site dir
        let source = if subdir.is_empty() {
            repo_dir_clone.clone()
        } else {
            repo_dir_clone.join(&subdir)
        };

        // Clear site dir and copy
        if site_dir_clone.exists() {
            std::fs::remove_dir_all(&site_dir_clone)?;
        }
        copy_dir_recursive(&source, &site_dir_clone)?;

        Ok((sha, message))
    })
    .await
    .map_err(|e| AppError::bad_request(format!("Git operation failed: {}", e)))?
    .map_err(|e| AppError::bad_request(format!("Git operation failed: {}", e)))?;

    // Get site ID for deployment record
    let site_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM sites WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;

    if let Some(site_id) = site_id {
        record_deployment(
            &state.db,
            &site_id,
            deployed_by,
            Some(&commit_sha),
            "success",
            None,
        )
        .await?;
    }

    sqlx::query(
        "UPDATE sites SET last_deployed_at = datetime('now'), updated_at = datetime('now'), last_commit_sha = ?, last_commit_message = ? WHERE slug = ?",
    )
    .bind(&commit_sha)
    .bind(&commit_message)
    .bind(slug)
    .execute(&state.db)
    .await?;

    tracing::info!(slug, commit = %commit_sha, "deployed from git");
    Ok(())
}

async fn record_deployment(
    db: &sqlx::SqlitePool,
    site_id: &str,
    deployed_by: &str,
    commit_sha: Option<&str>,
    status: &str,
    error_message: Option<&str>,
) -> Result<(), AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO deployments (id, site_id, deployed_by, commit_sha, status, error_message)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(site_id)
    .bind(deployed_by)
    .bind(commit_sha)
    .bind(status)
    .bind(error_message)
    .execute(db)
    .await?;
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Skip .git directory
        if src_path.file_name().is_some_and(|n| n == ".git") {
            continue;
        }

        // Skip symlinks entirely
        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
