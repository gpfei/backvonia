//! Authentication middleware for IAP verification
//!
//! This middleware extracts IAP headers, verifies receipts with Apple/Google,
//! and stores the verified purchase identity and tier in request extensions.

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    models::common::{IAPPlatform, PurchaseTier},
};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::Response,
};

/// Request extension storing verified IAP identity
#[derive(Debug, Clone)]
pub struct IAPIdentity {
    pub purchase_identity: String,
    pub purchase_tier: PurchaseTier,
    pub platform: IAPPlatform,
}

/// Authentication middleware that verifies IAP receipts
///
/// Extracts X-IAP-Platform and X-IAP-Receipt headers, verifies the receipt,
/// and stores the verified identity in request extensions.
///
/// Returns 401 Unauthorized if headers are missing or receipt verification fails.
pub async fn iap_auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response> {
    let headers = request.headers();

    // Extract IAP headers
    let platform_str = headers
        .get("x-iap-platform")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing X-IAP-Platform header".to_string()))?;

    let receipt = headers
        .get("x-iap-receipt")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing X-IAP-Receipt header".to_string()))?;

    // Parse platform
    let platform = IAPPlatform::from_str(platform_str)
        .ok_or_else(|| ApiError::BadRequest("Invalid IAP platform".to_string()))?;

    // Verify receipt with IAP service
    let verification = state.iap_service.verify_receipt(platform, receipt).await?;

    // Store verified identity in request extensions
    let identity = IAPIdentity {
        purchase_identity: verification.purchase_identity,
        purchase_tier: verification.purchase_tier,
        platform,
    };

    request.extensions_mut().insert(identity);

    // Continue to next middleware/handler
    Ok(next.run(request).await)
}

/// Axum extractor for IAP identity
///
/// Automatically extracts the verified IAP identity from request extensions.
/// Only works on routes protected by iap_auth_middleware.
impl<S> FromRequestParts<S> for IAPIdentity
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
            .get::<IAPIdentity>()
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "IAPIdentity not found - auth middleware not applied"
                ))
            })
    }
}
