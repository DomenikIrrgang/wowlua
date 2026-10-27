//! The pages' stylesheets, script and fonts, built into the server so a
//! registry needs nothing from wowlua.dev. `tokens.css`, `design-system.css`
//! and `chrome.css` are copies of the website's (`tests/assets.rs` compares
//! them when the website is checked out); `registry.css` is the registry's
//! own.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Each file: its name under `/assets/`, its type and its bytes.
pub const FILES: &[(&str, &str, &[u8])] = &[
    ("tokens.css", CSS, include_bytes!("../assets/tokens.css")),
    (
        "design-system.css",
        CSS,
        include_bytes!("../assets/design-system.css"),
    ),
    ("chrome.css", CSS, include_bytes!("../assets/chrome.css")),
    (
        "registry.css",
        CSS,
        include_bytes!("../assets/registry.css"),
    ),
    (
        "registry.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../assets/registry.js"),
    ),
    (
        "fonts/MartianMono-StdRg.woff2",
        WOFF2,
        include_bytes!("../assets/fonts/MartianMono-StdRg.woff2"),
    ),
    (
        "fonts/MartianMono-StdMd.woff2",
        WOFF2,
        include_bytes!("../assets/fonts/MartianMono-StdMd.woff2"),
    ),
    (
        "fonts/MartianMono-StdBd.woff2",
        WOFF2,
        include_bytes!("../assets/fonts/MartianMono-StdBd.woff2"),
    ),
    (
        "fonts/MartianMono-StdxBd.woff2",
        WOFF2,
        include_bytes!("../assets/fonts/MartianMono-StdxBd.woff2"),
    ),
    (
        "fonts/IBMPlexMono-Regular.ttf",
        TTF,
        include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    ),
    (
        "fonts/IBMPlexMono-Italic.ttf",
        TTF,
        include_bytes!("../assets/fonts/IBMPlexMono-Italic.ttf"),
    ),
    (
        "fonts/IBMPlexMono-Medium.ttf",
        TTF,
        include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    ),
    (
        "fonts/IBMPlexMono-SemiBold.ttf",
        TTF,
        include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    ),
    (
        "fonts/MartianMono-OFL.txt",
        TEXT,
        include_bytes!("../assets/fonts/MartianMono-OFL.txt"),
    ),
    (
        "fonts/IBMPlexMono-OFL.txt",
        TEXT,
        include_bytes!("../assets/fonts/IBMPlexMono-OFL.txt"),
    ),
];

const CSS: &str = "text/css; charset=utf-8";
const WOFF2: &str = "font/woff2";
const TTF: &str = "font/ttf";
const TEXT: &str = "text/plain; charset=utf-8";

/// The query that makes changed files new URLs: pages link
/// `/assets/<file>?v=<hash of all files>`, so the files can be cached for
/// long.
pub fn version() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for (name, _, bytes) in FILES {
            hasher.update(name.as_bytes());
            hasher.update(bytes);
        }
        crate::auth::hex(&hasher.finalize()[..6])
    })
}

/// `GET /assets/<file>` (also `fonts/<file>`).
pub async fn serve(Path(path): Path<String>) -> Response {
    match FILES.iter().find(|(name, _, _)| *name == path) {
        Some((_, kind, bytes)) => (
            [
                (header::CONTENT_TYPE, *kind),
                (header::CACHE_CONTROL, "public, max-age=604800"),
            ],
            *bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such file").into_response(),
    }
}
