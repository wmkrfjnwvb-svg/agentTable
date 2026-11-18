//! The web app, embedded in the binary.
//!
//! A release build carries `apps/web/dist` inside the executable, so one file is the whole
//! product. A debug build reads the folder from disk, so the web app can be rebuilt without
//! recompiling the server. When the web app was not built, a short page says how to build it.

use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../apps/web/dist"]
#[allow_missing = true]
struct Assets;

const MISSING: &str = "<!doctype html><title>agenttable</title><body style=\"font-family:sans-serif;padding:2rem\">\
<h1>The web app was not built</h1><p>Run <code>npm ci &amp;&amp; npm run build</code> in <code>apps/web</code>, \
then start the server again. The API is available under <a href=\"/api/status\">/api/status</a>.</p></body>";

/// Serves a file of the web app, or the app's entry page for client-side routes.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = if path.is_empty() { "index.html" } else { path };
    if let Some(found) = Assets::get(file) {
        let mime = found.metadata.mimetype().to_owned();
        let cache = if file.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, cache.to_owned()),
            ],
            found.data,
        )
            .into_response();
    }
    if path.starts_with("api/") || path.starts_with("assets/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match Assets::get("index.html") {
        Some(index) => Html(index.data).into_response(),
        None => Html(MISSING).into_response(),
    }
}
