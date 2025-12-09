use axum::{extract::State, Json};
use tracing::instrument;

use crate::{
    app_state::AppState,
    error::{ApiError, AppJson, Result},
    middleware::UserIdentity,
    models::{
        ai::{
            AIImageGenerateRequest, AIImageGenerateResponse, AITextContinueRequest,
            AITextContinueResponse, AITextEditMode, AITextEditRequest, AITextEditResponse,
            AITextIdeasRequest, AITextSummarizeRequest, AITextSummarizeResponse,
        },
        common::AIOperation,
    },
};

/// POST /api/v1/ai/text/continue
#[instrument(skip(state, identity, request))]
pub async fn text_continue(
    State(state): State<AppState>,
    identity: UserIdentity,
    AppJson(request): AppJson<AITextContinueRequest>,
) -> Result<Json<AITextContinueResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    // Ensure at least one node has content or summary
    let has_content = request.path_nodes.iter().any(|node| {
        !node.content.is_empty() || node.summary.as_ref().map_or(false, |s| !s.is_empty())
    });
    if !has_content {
        return Err(ApiError::BadRequest(
            "At least one node must have content or summary".to_string(),
        ));
    }

    let tier = &identity.account_tier;

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, AIOperation::ContinueProse)
        .await?;

    // Generate prose continuations using JSON-structured output
    let candidates = state
        .ai_service
        .generate_prose_continuations(
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
            request.instructions.as_deref(),
            tier,
        )
        .await?;

    Ok(Json(AITextContinueResponse { candidates }))
}

/// POST /api/v1/ai/image/generate
#[instrument(skip(state, identity, request))]
pub async fn image_generate(
    State(state): State<AppState>,
    identity: UserIdentity,
    AppJson(request): AppJson<AIImageGenerateRequest>,
) -> Result<Json<AIImageGenerateResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let tier = &identity.account_tier;

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

    Ok(Json(AIImageGenerateResponse { image }))
}

/// POST /api/v1/ai/text/edit
#[instrument(skip(state, identity, request))]
pub async fn text_edit(
    State(state): State<AppState>,
    identity: UserIdentity,
    AppJson(request): AppJson<AITextEditRequest>,
) -> Result<Json<AITextEditResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    let tier = &identity.account_tier;

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
            tier,
        )
        .await?;

    Ok(Json(AITextEditResponse {
        mode: request.mode,
        candidates,
    }))
}

/// POST /api/v1/ai/text/ideas
#[instrument(skip(state, identity, request))]
pub async fn text_ideas(
    State(state): State<AppState>,
    identity: UserIdentity,
    AppJson(request): AppJson<AITextIdeasRequest>,
) -> Result<Json<AITextContinueResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    // Ensure at least one node has content or summary
    let has_content = request.path_nodes.iter().any(|node| {
        !node.content.is_empty() || node.summary.as_ref().map_or(false, |s| !s.is_empty())
    });
    if !has_content {
        return Err(ApiError::BadRequest(
            "At least one node must have content or summary".to_string(),
        ));
    }

    let tier = &identity.account_tier;

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, AIOperation::ContinueIdeas)
        .await?;

    // Generate continuation ideas using JSON-structured output
    let candidates = state
        .ai_service
        .generate_continuation_ideas(
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
            request.instructions.as_deref(),
            tier,
        )
        .await?;

    Ok(Json(AITextContinueResponse { candidates }))
}

/// POST /api/v1/ai/text/summarize
#[instrument(skip(state, identity, request))]
pub async fn text_summarize(
    State(state): State<AppState>,
    identity: UserIdentity,
    AppJson(request): AppJson<AITextSummarizeRequest>,
) -> Result<Json<AITextSummarizeResponse>> {
    // Validate request
    use validator::Validate;
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    if request.nodes.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one node required".to_string(),
        ));
    }

    if request.nodes.len() > 20 {
        return Err(ApiError::BadRequest(
            "Maximum 20 nodes per request".to_string(),
        ));
    }

    let tier = &identity.account_tier;

    // Atomically check and increment quota - 1 credit per batch
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, AIOperation::Summarize)
        .await?;

    // Generate summaries
    let summaries = state
        .ai_service
        .generate_summaries(request.story_context.as_ref(), &request.nodes, tier)
        .await?;

    Ok(Json(AITextSummarizeResponse { summaries }))
}
