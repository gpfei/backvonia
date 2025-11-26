use axum::{extract::State, Json};
use tracing::instrument;
use validator::Validate;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    middleware::IAPIdentity,
    models::iap::{IAPVerifyData, IAPVerifyRequest, IAPVerifyResponse, QuotaData, QuotaResponse},
};

/// POST /api/v1/iap/verify
#[instrument(skip(state, request))]
pub async fn verify_iap(
    State(state): State<AppState>,
    Json(request): Json<IAPVerifyRequest>,
) -> Result<Json<IAPVerifyResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    // Verify the receipt
    let verification = state
        .iap_service
        .verify_receipt(request.platform, &request.receipt)
        .await?;

    Ok(Json(IAPVerifyResponse {
        success: true,
        data: IAPVerifyData {
            purchase_tier: verification.purchase_tier,
            purchase_identity: verification.purchase_identity,
            product_id: verification.product_id,
            valid_until: verification.valid_until,
            platform: verification.platform,
        },
    }))
}

/// GET /api/v1/quota
#[instrument(skip(state, identity))]
pub async fn get_quota(
    State(state): State<AppState>,
    identity: IAPIdentity,
) -> Result<Json<QuotaResponse>> {
    // Get quota info
    let quota = state
        .quota_service
        .get_quota_info(&identity.purchase_identity, identity.purchase_tier)
        .await?;

    Ok(Json(QuotaResponse {
        success: true,
        data: QuotaData {
            purchase_tier: identity.purchase_tier,
            quota,
        },
    }))
}
