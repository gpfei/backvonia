use axum::{extract::State, Json};
use tracing::instrument;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    middleware::IAPIdentity,
    models::ai::{
        AIImageGenerateData, AIImageGenerateRequest, AIImageGenerateResponse, AITextContinueData,
        AITextContinueRequest, AITextContinueResponse,
    },
};

/// POST /api/v1/ai/text/continue
#[instrument(skip(state, identity, request))]
pub async fn text_continue(
    State(state): State<AppState>,
    identity: IAPIdentity,
    Json(request): Json<AITextContinueRequest>,
) -> Result<Json<AITextContinueResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let purchase_identity = &identity.purchase_identity;
    let tier = identity.purchase_tier;

    // Atomically check and increment quota (prevents race conditions)
    let quota_status = state
        .quota_service
        .check_and_increment_text_quota(&purchase_identity, tier)
        .await?;

    // Generate candidates
    let candidates = state
        .ai_service
        .generate_text_continuations(
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
        )
        .await?;

    // Build quota subset for response
    let quota = crate::models::common::QuotaSubset {
        text_remaining_today: quota_status.text_limit - quota_status.text_used,
        image_remaining_today: quota_status.image_limit - quota_status.image_used,
    };

    Ok(Json(AITextContinueResponse {
        success: true,
        data: AITextContinueData {
            purchase_tier: tier,
            quota,
            candidates,
        },
    }))
}

/// POST /api/v1/ai/image/generate
#[instrument(skip(state, identity, request))]
pub async fn image_generate(
    State(state): State<AppState>,
    identity: IAPIdentity,
    Json(request): Json<AIImageGenerateRequest>,
) -> Result<Json<AIImageGenerateResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let purchase_identity = &identity.purchase_identity;
    let tier = identity.purchase_tier;

    // Atomically check and increment quota (prevents race conditions)
    let quota_status = state
        .quota_service
        .check_and_increment_image_quota(&purchase_identity, tier)
        .await?;

    // Generate image
    let image = state
        .ai_service
        .generate_image(&request.story_context, &request.node, &request.image_params)
        .await?;

    // Build quota subset for response
    let quota = crate::models::common::QuotaSubset {
        text_remaining_today: quota_status.text_limit - quota_status.text_used,
        image_remaining_today: quota_status.image_limit - quota_status.image_used,
    };

    Ok(Json(AIImageGenerateResponse {
        success: true,
        data: AIImageGenerateData {
            purchase_tier: tier,
            quota,
            image,
        },
    }))
}
