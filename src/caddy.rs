use std::sync::Arc;

use crate::AppState;

pub async fn generate_caddyfile(state: &Arc<AppState>) -> anyhow::Result<String> {
    let external_url = &state.config.external_url;
    let bind_addr = &state.config.bind_addr;

    // Parse the app's port from bind_addr
    let app_port = bind_addr
        .rsplit(':')
        .next()
        .unwrap_or("8080");
    let app_upstream = format!("localhost:{}", app_port);

    // Get sites with custom domains
    #[derive(sqlx::FromRow)]
    struct SiteRow {
        slug: String,
        custom_domain: Option<String>,
    }

    let sites = sqlx::query_as::<_, SiteRow>(
        "SELECT slug, custom_domain FROM sites WHERE custom_domain IS NOT NULL AND custom_domain != ''",
    )
    .fetch_all(&state.db)
    .await?;

    // Parse the primary domain from external_url
    let primary_domain = external_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let mut caddyfile = String::new();

    // Global options
    caddyfile.push_str("{\n    admin :2019\n}\n\n");

    // Primary domain block
    caddyfile.push_str(&format!("{} {{\n", primary_domain));

    // Management UI, API, and auth routes — proxy to app
    caddyfile.push_str(&format!(
        "    handle /api/* {{\n        reverse_proxy {}\n    }}\n\n",
        app_upstream
    ));
    caddyfile.push_str(&format!(
        "    handle /auth/* {{\n        reverse_proxy {}\n    }}\n\n",
        app_upstream
    ));
    caddyfile.push_str(&format!(
        "    handle /login {{\n        reverse_proxy {}\n    }}\n\n",
        app_upstream
    ));
    caddyfile.push_str(&format!(
        "    handle /assets/* {{\n        reverse_proxy {}\n    }}\n\n",
        app_upstream
    ));

    // Static sites — protected by forward auth
    caddyfile.push_str(&format!(
        "    handle /s/* {{\n\
         \x20       forward_auth {} {{\n\
         \x20           uri /auth/verify\n\
         \x20           @unauthorized status 401\n\
         \x20           handle_response @unauthorized {{\n\
         \x20               redir /login?redirect={{http.request.uri}}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       uri strip_prefix /s\n\
         \x20       file_server {{\n\
         \x20           root {}\n\
         \x20       }}\n\
         \x20   }}\n\n",
        app_upstream, state.config.sites_dir
    ));

    // Default — dashboard
    caddyfile.push_str(&format!(
        "    handle {{\n        reverse_proxy {}\n    }}\n",
        app_upstream
    ));
    caddyfile.push_str("}\n");

    // Custom domain blocks
    for site in &sites {
        if let Some(ref domain) = site.custom_domain {
            let site_root =
                format!("{}/{}", state.config.sites_dir, site.slug);
            caddyfile.push_str(&format!(
                "\n{} {{\n\
                 \x20   forward_auth {} {{\n\
                 \x20       uri /auth/verify\n\
                 \x20       @unauthorized status 401\n\
                 \x20       handle_response @unauthorized {{\n\
                 \x20           redir https://{}/login?redirect=https://{domain}{{http.request.uri}}\n\
                 \x20       }}\n\
                 \x20   }}\n\
                 \x20   file_server {{\n\
                 \x20       root {}\n\
                 \x20   }}\n\
                 }}\n",
                domain, app_upstream, primary_domain, site_root
            ));
        }
    }

    Ok(caddyfile)
}

pub async fn reload_caddy(state: &Arc<AppState>) -> anyhow::Result<()> {
    let caddyfile_content = generate_caddyfile(state).await?;

    let caddyfile_path = format!("{}/Caddyfile", state.config.caddy_root);
    tokio::fs::create_dir_all(&state.config.caddy_root).await?;
    tokio::fs::write(&caddyfile_path, &caddyfile_content).await?;

    let output = tokio::process::Command::new(&state.config.caddy_bin)
        .args(["reload", "--config", &caddyfile_path, "--adapter", "caddyfile"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("caddy reload failed: {}", stderr);
        anyhow::bail!("caddy reload failed: {}", stderr);
    }

    tracing::info!("caddy config reloaded");
    Ok(())
}
