//! `wowlua-registry`: serves the registry on plain HTTP, configured by the
//! environment (see the crate's README). `wowlua-registry health` asks a
//! running server's `/health` (the container's health check).

use std::path::PathBuf;
use wowlua_registry::{Config, auth};

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn config() -> Result<(Config, String), String> {
    let addr = var("WOWLUA_REGISTRY_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into());
    let public_url = var("WOWLUA_REGISTRY_URL").unwrap_or_default();
    let required = |name: &str| var(name).ok_or_else(|| format!("set {name}"));
    let max_archive = match var("WOWLUA_REGISTRY_MAX_ARCHIVE") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| "WOWLUA_REGISTRY_MAX_ARCHIVE is a number of bytes".to_string())?,
        None => 10 * 1024 * 1024,
    };
    let rate = |name: &str, default: Option<wowlua_registry::limits::Rate>| match var(name) {
        Some(v) => wowlua_registry::limits::Rate::parse(&v).map_err(|e| format!("{name}: {e}")),
        None => Ok(default),
    };
    let defaults = wowlua_registry::limits::Limits::default();
    let limits = wowlua_registry::limits::Limits {
        publish: rate("WOWLUA_REGISTRY_RATE_PUBLISH", defaults.publish)?,
        login: rate("WOWLUA_REGISTRY_RATE_LOGIN", defaults.login)?,
        write: rate("WOWLUA_REGISTRY_RATE_WRITE", defaults.write)?,
        read: rate("WOWLUA_REGISTRY_RATE_READ", defaults.read)?,
        trusted_proxies: var("WOWLUA_REGISTRY_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter(|p| !p.trim().is_empty())
            .map(|p| {
                wowlua_registry::limits::Net::parse(p)
                    .map_err(|e| format!("WOWLUA_REGISTRY_TRUSTED_PROXIES: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let config = Config {
        public_url,
        data_dir: PathBuf::from(var("WOWLUA_REGISTRY_DATA").unwrap_or_else(|| "/data".into())),
        github: auth::GitHub {
            client_id: required("WOWLUA_REGISTRY_GITHUB_CLIENT_ID")?,
            client_secret: required("WOWLUA_REGISTRY_GITHUB_CLIENT_SECRET")?,
            url: var("WOWLUA_REGISTRY_GITHUB_URL").unwrap_or_else(|| "https://github.com".into()),
            api_url: var("WOWLUA_REGISTRY_GITHUB_API_URL")
                .unwrap_or_else(|| "https://api.github.com".into()),
        },
        admins: var("WOWLUA_REGISTRY_ADMINS")
            .unwrap_or_default()
            .split(',')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect(),
        max_archive,
        limits,
    };
    Ok((config, addr))
}

fn health() -> bool {
    let addr = var("WOWLUA_REGISTRY_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into());
    let port = addr.rsplit(':').next().unwrap_or("8080");
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build(),
    );
    agent
        .get(&format!("http://127.0.0.1:{port}/health"))
        .call()
        .is_ok()
}

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("health") {
        return if health() {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
    let (config, addr) = match config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wowlua-registry: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    if config.public_url.is_empty() {
        eprintln!("wowlua-registry: set WOWLUA_REGISTRY_URL (e.g. https://registry.wowlua.dev)");
        return std::process::ExitCode::from(2);
    }
    let running = match wowlua_registry::spawn(config.clone(), &addr) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wowlua-registry: cannot start on {addr}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    eprintln!(
        "wowlua-registry {} serving {} on {} (data in {})",
        env!("CARGO_PKG_VERSION"),
        config.public_url,
        running.url,
        config.data_dir.display()
    );
    // Until SIGTERM (`docker stop`) or Ctrl-C; then finish the requests.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wowlua-registry: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            match signal(SignalKind::terminate()) {
                Ok(mut term) => {
                    tokio::select! {
                        _ = term.recv() => {}
                        _ = tokio::signal::ctrl_c() => {}
                    }
                }
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    });
    eprintln!("wowlua-registry: stopping");
    running.stop();
    std::process::ExitCode::SUCCESS
}
