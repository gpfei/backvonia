// Route modules
pub mod ai;
pub mod credits;
pub mod iap;

use crate::{
    app_state::AppState,
    middleware::{create_rate_limiter, iap_auth_middleware},
};
use axum::{
    middleware,
    routing::{get, post},
    Router,
};

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .nest("/api/v1", api_v1_routes(state.clone()))
        .with_state(state)
}

/// API v1 routes
fn api_v1_routes(state: AppState) -> Router<AppState> {
    // Protected routes requiring both authentication and rate limiting
    let rate_limiter = create_rate_limiter(state.redis.clone());
    let protected_routes = Router::new()
        .route("/ai/text/continue", post(ai::text_continue))
        .route("/ai/text/edit", post(ai::text_edit))
        .route("/ai/image/generate", post(ai::image_generate))
        .route_layer(middleware::from_fn(rate_limiter))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            iap_auth_middleware,
        ));

    // Auth-only routes (no rate limiting)
    let auth_only_routes = Router::new()
        .route("/quota", get(credits::get_credits_quota)) // Returns credits-aware quota
        .route("/credits/purchase", post(credits::record_credit_purchase))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            iap_auth_middleware,
        ));

    // Public routes (no authentication required)
    let public_routes = Router::new().route("/iap/verify", post(iap::verify_iap));

    // Combine all routes
    Router::new()
        .merge(protected_routes)
        .merge(auth_only_routes)
        .merge(public_routes)
}
