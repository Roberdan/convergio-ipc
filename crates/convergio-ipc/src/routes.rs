//! HTTP API routes for convergio-ipc.

use axum::Router;

/// Returns the router for this crate's API endpoints.
pub fn routes() -> Router {
    Router::new()
    // .route("/api/ipc/health", get(health))
}
