//! The wowlua registry server (spec Package manager §10.10, Publishing
//! §11.8): the sparse index with ETags, downloads, publishing, yanks and
//! owners, with accounts from GitHub sign-in and hashed API tokens.
//!
//! It speaks plain HTTP: HTTPS ends at the proxy in front of it. The data
//! lives in one directory (see [`store`]). The HTTP stack is axum on tokio
//! and hyper, the maintained standard of Rust's HTTP servers; the protocol
//! code itself is synchronous and shared with the client (`wowlua-pkg`'s
//! `registry` module and `wowlua-manifest`'s validation).

pub mod api;
pub mod auth;
pub mod pages;
pub mod store;
#[doc(hidden)]
pub mod testing;

use std::path::PathBuf;
use std::sync::Arc;

/// The server's settings (the environment variables of `main.rs`).
#[derive(Clone, Debug)]
pub struct Config {
    /// The public URL (`https://registry.wowlua.dev`), without a trailing
    /// `/`: what `config.json` names and GitHub redirects back to.
    pub public_url: String,
    pub data_dir: PathBuf,
    pub github: auth::GitHub,
    /// GitHub logins allowed to assign reserved names and to act as an
    /// owner of every package.
    pub admins: Vec<String>,
    /// The largest archive a publish may upload, in bytes (413 beyond).
    pub max_archive: usize,
}

impl Config {
    pub fn is_admin(&self, login: &str) -> bool {
        self.admins.iter().any(|a| store::same_login(a, login))
    }

    /// Whether cookies need `Secure` (the public URL is HTTPS).
    fn secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}

/// What every request sees.
pub struct App {
    pub config: Config,
    pub store: store::Store,
    pub sessions: auth::Sessions,
}

pub type Shared = Arc<App>;

/// The routes.
pub fn router(app: Shared) -> axum::Router {
    use axum::routing::{get, post, put};
    let body_limit = app.config.max_archive + 1024 * 1024;
    axum::Router::new()
        .route("/", get(pages::home))
        .route("/health", get(|| async { "ok" }))
        .route("/config.json", get(api::config))
        .route("/index/{name}", get(api::index))
        .route("/dl/{name}/{file}", get(api::download))
        .route(
            "/v1/packages/{name}/owners",
            get(api::owners_list)
                .put(api::owners_add)
                .delete(api::owners_remove),
        )
        .route(
            "/v1/packages/{name}/{version}",
            put(api::publish)
                .delete(api::yank)
                .layer(axum::extract::DefaultBodyLimit::max(body_limit)),
        )
        .route("/v1/packages/{name}/{version}/unyank", put(api::unyank))
        .route("/v1/reserved", get(api::reserved_list))
        .route(
            "/v1/reserved/{name}",
            put(api::reserved_set).delete(api::reserved_release),
        )
        .route("/login", get(pages::login))
        .route("/login/github", get(pages::login_github))
        .route("/login/callback", get(pages::callback))
        .route("/logout", post(pages::logout))
        .route("/account", get(pages::account))
        .route("/account/tokens", post(pages::token_create))
        .route("/account/tokens/revoke", post(pages::token_revoke))
        .route("/account/yank", post(pages::yank))
        .route("/account/owners", post(pages::owners))
        .route("/account/reserved", post(pages::reserved))
        .fallback(api::not_found)
        .with_state(app)
}

/// A running server (tests, and `main`).
pub struct Running {
    /// `http://<address>`.
    pub url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Stops the server and waits for it.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        if let Some(s) = self.shutdown.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt();
    }
}

/// Opens the data directory and serves on `addr` in a thread of its own.
/// An empty `public_url` becomes the address it listens on.
pub fn spawn(mut config: Config, addr: &str) -> std::io::Result<Running> {
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    let url = format!("http://{local}");
    if config.public_url.is_empty() {
        config.public_url = url.clone();
    }
    config.public_url = wowlua_pkg::registry::normalize_url(&config.public_url);
    let store = store::Store::open(&config.data_dir)?;
    let app = Arc::new(App {
        config,
        store,
        sessions: auth::Sessions::default(),
    });
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let thread = std::thread::spawn(move || {
        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("wowlua-registry: {e}");
                    return;
                }
            };
            let served = axum::serve(listener, router(app))
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
            if let Err(e) = served {
                eprintln!("wowlua-registry: {e}");
            }
        });
    });
    Ok(Running {
        url,
        shutdown: Some(tx),
        thread: Some(thread),
    })
}
