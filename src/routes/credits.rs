use axum::{extract::State, Json};
use tracing::instrument;
use validator::Validate;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    middleware::UserIdentity,
    models::credits::{CreditPurchaseRequest, CreditPurchaseResponse, CreditsQuotaResponse},
};

/// POST /api/v1/credits/purchase
#[instrument(skip(state, request))]
pub async fn record_credit_purchase(
    State(state): State<AppState>,
    identity: UserIdentity,
    Json(request): Json<CreditPurchaseRequest>,
) -> Result<Json<CreditPurchaseResponse>> {
    // Validate request
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    // Extract credit amount from product_id
    let amount = request.extract_credit_amount().ok_or_else(|| {
        ApiError::BadRequest(format!("Invalid product_id: {}", request.product_id))
    })?;

    // Record the purchase
    let (purchase_id, total_extra) = state
        .credits_service
        .record_purchase(
            identity.user_id,
            request.original_transaction_id.as_deref(),
            &request.transaction_id,
            &request.product_id,
            request.platform,
            amount,
            request.purchase_date,
            request.receipt.as_deref(),
        )
        .await?;

    // Get updated quota info
    let quota_info = state
        .credits_service
        .get_credits_quota_summary(identity.user_id)
        .await?;

    Ok(Json(CreditPurchaseResponse {
        credits_added: amount,
        total_extra_credits: total_extra,
        purchase_id,
        quota: quota_info,
    }))
}

/// GET /api/v1/quota
#[instrument(skip(state, identity))]
pub async fn get_credits_quota(
    State(state): State<AppState>,
    identity: UserIdentity,
) -> Result<Json<CreditsQuotaResponse>> {
    // Get credits quota info
    let quota_info = state
        .credits_service
        .get_credits_quota(identity.user_id)
        .await?;

    Ok(Json(CreditsQuotaResponse {
        account_tier: identity.account_tier,
        subscription_credits: quota_info.subscription_credits.clone(),
        extra_credits: quota_info.extra_credits.clone(),
        total_credits: quota_info.total_credits,
    }))
}
