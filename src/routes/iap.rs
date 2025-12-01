use axum::{extract::State, Json};
use tracing::instrument;
use validator::Validate;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    models::iap::{IAPVerifyData, IAPVerifyRequest, IAPVerifyResponse},
};

/// POST /api/v1/iap/verify
#[instrument(skip(state, request))]
pub async fn verify_iap(
    State(state): State<AppState>,
    Json(request): Json<IAPVerifyRequest>,
) -> Result<Json<IAPVerifyResponse>> {
    // Validate request
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
