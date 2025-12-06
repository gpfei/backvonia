use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    services::jwt_service::JWTService,
};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::Response,
};
use entity::sea_orm_active_enums::AccountTier;
use uuid::Uuid;

/// Request extension storing verified user identity from JWT
#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub user_id: Uuid,
    pub account_tier: AccountTier,
}

/// JWT authentication middleware
///
/// Extracts the Authorization header, validates the JWT access token,
/// and stores the verified user identity in request extensions.
///
/// Returns 401 Unauthorized if the header is missing or token validation fails.
pub async fn jwt_auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response> {
    let headers = request.headers();

    // Extract Authorization header
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;

    // Parse "Bearer <token>" format
    let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        ApiError::InvalidToken(
            "Invalid Authorization format, expected 'Bearer <token>'".to_string(),
        )
    })?;

    // Validate JWT token
    let claims = state.jwt_service.validate_token(token)?;

    // Extract user_id and account_tier from claims
    let user_id = JWTService::user_id_from_claims(&claims)?;
    let account_tier = JWTService::account_tier_from_claims(&claims)?;

    // Store verified identity in request extensions
    let identity = UserIdentity {
        user_id,
        account_tier,
    };

    request.extensions_mut().insert(identity);

    // Continue to next middleware/handler
    Ok(next.run(request).await)
}

/// Axum extractor for user identity
///
/// Automatically extracts the verified user identity from request extensions.
/// Only works on routes protected by jwt_auth_middleware.
impl<S> FromRequestParts<S> for UserIdentity
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<UserIdentity>()
            .cloned()
            .ok_or_else(|| {
                ApiError::Unauthorized(
                    "User identity not found - route must be protected by jwt_auth_middleware"
                        .to_string(),
                )
            })
    }
}
