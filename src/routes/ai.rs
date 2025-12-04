use axum::{extract::State, Json};
use tracing::instrument;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    middleware::UserIdentity,
    models::{
        ai::{
            AIImageGenerateData, AIImageGenerateRequest, AIImageGenerateResponse,
            AITextContinueData, AITextContinueMode, AITextContinueRequest, AITextContinueResponse,
            AITextEditData, AITextEditMode, AITextEditRequest, AITextEditResponse,
        },
        common::AIOperation,
    },
};

/// POST /api/v1/ai/text/continue
#[instrument(skip(state, identity, request))]
pub async fn text_continue(
    State(state): State<AppState>,
    identity: UserIdentity,
    Json(request): Json<AITextContinueRequest>,
) -> Result<Json<AITextContinueResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let tier = identity.account_tier;

    // Determine operation based on mode
    let operation = match request.mode {
        AITextContinueMode::Prose => AIOperation::ContinueProse,
        AITextContinueMode::Ideas => AIOperation::ContinueIdeas,
    };

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, operation)
        .await?;

    // Generate candidates
    let candidates = state
        .ai_service
        .generate_text_continuations(
            request.mode,
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
        )
        .await?;

    Ok(Json(AITextContinueResponse {
        success: true,
        data: AITextContinueData { candidates },
    }))
}

/// POST /api/v1/ai/image/generate
#[instrument(skip(state, identity, request))]
pub async fn image_generate(
    State(state): State<AppState>,
    identity: UserIdentity,
    Json(request): Json<AIImageGenerateRequest>,
) -> Result<Json<AIImageGenerateResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let tier = identity.account_tier;

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, AIOperation::ImageGenerate)
        .await?;

    // Generate image
    let image = state
        .ai_service
        .generate_image(&request.story_context, &request.node, &request.image_params)
        .await?;

    Ok(Json(AIImageGenerateResponse {
        success: true,
        data: AIImageGenerateData { image },
    }))
}

/// POST /api/v1/ai/text/edit
#[instrument(skip(state, identity, request))]
pub async fn text_edit(
    State(state): State<AppState>,
    identity: UserIdentity,
    Json(request): Json<AITextEditRequest>,
) -> Result<Json<AITextEditResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let tier = identity.account_tier;

    // Determine operation based on edit mode
    let operation = match request.mode {
        AITextEditMode::Expand => AIOperation::EditExpand,
        AITextEditMode::Shorten => AIOperation::EditShorten,
        AITextEditMode::Rewrite => AIOperation::EditRewrite,
        AITextEditMode::FixGrammar => AIOperation::EditFixGrammar,
    };

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, operation)
        .await?;

    // Generate edit candidates
    let candidates = state
        .ai_service
        .generate_text_edit(
            request.mode,
            request.story_context.as_ref(),
            &request.input,
            &request.edit_params,
        )
        .await?;

    Ok(Json(AITextEditResponse {
        success: true,
        data: AITextEditData {
            mode: request.mode,
            candidates,
        },
    }))
}
