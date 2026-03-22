use crate::AppState;

pub fn generate_caddyfile(state: &AppState) -> String {
    let app_port = state
        .config
        .bind_addr
        .rsplit(':')
        .next()
        .unwrap_or("8080");
    let app_upstream = format!("localhost:{}", app_port);

    let primary_domain = state
        .config
        .external_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    format!(
        r#"{{
    admin :2019
}}

{primary_domain} {{
    handle /api/* {{
        reverse_proxy {app_upstream}
    }}

    handle /auth/* {{
        reverse_proxy {app_upstream}
    }}

    handle /login {{
        reverse_proxy {app_upstream}
    }}

    handle /assets/* {{
        reverse_proxy {app_upstream}
    }}

    handle /s/* {{
        forward_auth {app_upstream} {{
            uri /auth/verify
            @unauthorized status 401
            handle_response @unauthorized {{
                redir /login?redirect={{http.request.uri}}
            }}
        }}
        uri strip_prefix /s
        file_server {{
            root {sites_dir}
        }}
    }}

    handle {{
        reverse_proxy {app_upstream}
    }}
}}
"#,
        sites_dir = state.config.sites_dir
    )
}

pub async fn reload_caddy(state: &AppState) -> anyhow::Result<()> {
    let caddyfile_content = generate_caddyfile(state);

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
