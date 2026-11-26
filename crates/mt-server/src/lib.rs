//! The agenttable server: one binary that serves the API and the web app.
//!
//! This is the end-to-end build of the product as it stands: the finished engines
//! (`mt-core`, `mt-refdata`, `mt-query`, `mt-formula`, `mt-perm`, `mt-audit`) behind an HTTP
//! API, with a demo purchasing workspace and the web app embedded. Parts that are not built
//! yet have clearly marked stand-ins: storage is one JSON file (until `mt-store`), identity is
//! a user switcher (until `mt-identity`), and lifecycles, automations, dashboards and AI show
//! as placeholders in the web app.
#![forbid(unsafe_code)]

pub mod api;
pub mod builder;
pub mod error;
pub mod formula;
pub mod model;
pub mod process;
pub mod seed;
pub mod view;
pub mod web;

use api::{App, Clock};
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;

/// Builds the application: the API routes and the web app.
pub fn router(app: Arc<App>) -> Router {
    api::routes().fallback(web::serve).with_state(app)
}

/// What a new workspace starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// One administrator, no bases: build your own, or install a template.
    Empty,
    /// The purchasing sample installed.
    Demo,
}

/// Loads the workspace from `data_file`, or creates a new one as `start` says.
///
/// # Errors
/// When the file exists but cannot be read, or the new workspace cannot be built.
pub fn open(
    data_file: Option<PathBuf>,
    clock: Clock,
    start: Start,
) -> Result<Arc<App>, error::ApiError> {
    let now = clock.now();
    let existing = data_file.as_ref().filter(|path| path.exists());
    let ws = match existing {
        Some(path) => {
            let text = std::fs::read(path)
                .map_err(|e| error::ApiError::internal(format!("{}: {e}", path.display())))?;
            let data: model::Data = serde_json::from_slice(&text)
                .map_err(|e| error::ApiError::internal(format!("{}: {e}", path.display())))?;
            let mut ws = model::Workspace::new(data)?;
            ws.recompute(now);
            ws
        }
        None => match start {
            Start::Empty => seed::empty()?,
            Start::Demo => seed::demo(now)?,
        },
    };
    let app = Arc::new(App {
        ws: tokio::sync::Mutex::new(ws),
        data_file,
        clock,
    });
    Ok(app)
}
