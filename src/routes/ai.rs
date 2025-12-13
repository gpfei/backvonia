use axum::{extract::State, Json};
use base64::Engine;
use tracing::instrument;

use crate::{
    app_state::AppState,
    error::{ApiError, AppJson, Result},
    middleware::UserIdentity,
    models::{
        ai::{
            AIImageGenerateRequest, AIImageGenerateResponse, AITextContinueRequest,
            AITextContinueResponse, AITextEditMode, AITextEditRequest, AITextEditResponse,
            AITextIdeasRequest, AITextSummarizeRequest, AITextSummarizeResponse, GeneratedImage,
        },
        common::AIOperation,
    },
};
use entity::ai_image_generation;
use sea_orm::{ActiveModelTrait, Set};
use uuid::Uuid;

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
        !node.content.is_empty() || node.summary.as_ref().is_some_and(|s| !s.is_empty())
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
    let generation_result = state
        .ai_service
        .generate_prose_continuations(
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
            request.instructions.as_deref(),
            tier,
        )
        .await;

    // Handle errors with credit refund
    match generation_result {
        Ok(candidates) => Ok(Json(AITextContinueResponse { candidates })),
        Err(err) => {
            // Refund credits after failed generation
            if let Err(refund_err) = state
                .quota_service
                .refund_quota_weighted(identity.user_id, tier, AIOperation::ContinueProse)
                .await
            {
                tracing::error!(
                    user_id = %identity.user_id,
                    error = %refund_err,
                    "Failed to refund credits after text generation failure - user may have lost credits"
                );
            } else {
                tracing::info!(
                    user_id = %identity.user_id,
                    "Successfully refunded credits after text generation failure"
                );
            }
            Err(err)
        }
    }
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

    // request
    //     .node
    //     .validate_has_content()
    //     .map_err(|msg| ApiError::BadRequest(msg.to_string()))?;

    let tier = &identity.account_tier;
    let start_time = std::time::Instant::now();

    // Atomically check and increment quota with weighted cost
    state
        .quota_service
        .check_and_increment_quota_weighted(identity.user_id, tier, AIOperation::ImageGenerate)
        .await?;

    // Attempt image generation with error tracking
    let generation_result = async {
        // Generate image (returns image bytes + metadata)
        let (image_bytes, image_metadata) = state
            .ai_service
            .generate_image(
                &request.story_context,
                &request.node,
                &request.image_params,
                tier,
            )
            .await?;

        // Encode to base64
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        let file_size = image_bytes.len();

        Ok::<_, ApiError>((base64_data, file_size, image_metadata))
    }
    .await;

    let generation_time_ms = start_time.elapsed().as_millis() as i32;

    // Handle result and save record
    match generation_result {
        Ok((base64_data, file_size, image_metadata)) => {
            // Save successful generation record
            let generation_record = ai_image_generation::ActiveModel {
                id: Set(Uuid::new_v4()),
                user_id: Set(identity.user_id),
                story_title: Set(request.story_context.title.clone()),
                node_summary: Set(request.node.summary.clone()),
                node_content: Set(request.node.content.clone()),
                style: Set(request
                    .image_params
                    .style
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "illustration".to_string())),
                resolution: Set(request.image_params.resolution.clone()),
                image_url: Set(String::new()), // No longer storing image URL
                temp_url: Set(None),
                temp_url_expires_at: Set(None),
                width: Set(image_metadata.width as i32),
                height: Set(image_metadata.height as i32),
                file_size_bytes: Set(Some(file_size as i32)),
                credits_used: Set(10),
                generation_time_ms: Set(Some(generation_time_ms)),
                ai_provider: Set(Some("openai-dalle3".to_string())),
                status: Set("success".to_string()),
                error_message: Set(None),
                created_at: Set(time::OffsetDateTime::now_utc()),
            };

            generation_record
                .insert(&state.db)
                .await
                .map_err(ApiError::Database)?;

            Ok(Json(AIImageGenerateResponse {
                image: GeneratedImage {
                    data: base64_data,
                    mime_type: image_metadata.mime_type,
                    width: image_metadata.width,
                    height: image_metadata.height,
                },
            }))
        }
        Err(err) => {
            // Save failed generation record for analytics
            let error_msg = err.to_string();
            tracing::error!("Image generation failed: {}", error_msg);

            let failed_record = ai_image_generation::ActiveModel {
                id: Set(Uuid::new_v4()),
                user_id: Set(identity.user_id),
                story_title: Set(request.story_context.title.clone()),
                node_summary: Set(request.node.summary.clone()),
                node_content: Set(request.node.content.clone()),
                style: Set(request
                    .image_params
                    .style
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "illustration".to_string())),
                resolution: Set(request.image_params.resolution.clone()),
                image_url: Set(String::new()),
                temp_url: Set(None),
                temp_url_expires_at: Set(None),
                width: Set(0),
                height: Set(0),
                file_size_bytes: Set(None),
                credits_used: Set(10), // Credits were deducted but will be refunded
                generation_time_ms: Set(Some(generation_time_ms)),
                ai_provider: Set(Some("openai-dalle3".to_string())),
                status: Set("failed".to_string()),
                error_message: Set(Some(error_msg.clone())),
                created_at: Set(time::OffsetDateTime::now_utc()),
            };

            // Save failed record (don't fail if this fails)
            if let Err(db_err) = failed_record.insert(&state.db).await {
                tracing::error!("Failed to save error record: {}", db_err);
            }

            // Refund credits after failed generation
            if let Err(refund_err) = state
                .quota_service
                .refund_quota_weighted(identity.user_id, tier, AIOperation::ImageGenerate)
                .await
            {
                tracing::error!(
                    user_id = %identity.user_id,
                    error = %refund_err,
                    "Failed to refund credits after generation failure - user may have lost credits"
                );
                // Don't fail the request - user already saw generation error
            } else {
                tracing::info!(
                    user_id = %identity.user_id,
                    "Successfully refunded 10 credits after generation failure"
                );
            }

            Err(err)
        }
    }
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
    let generation_result = state
        .ai_service
        .generate_text_edit(
            request.mode,
            request.story_context.as_ref(),
            &request.input,
            &request.edit_params,
            tier,
        )
        .await;

    // Handle errors with credit refund
    match generation_result {
        Ok(candidates) => Ok(Json(AITextEditResponse {
            mode: request.mode,
            candidates,
        })),
        Err(err) => {
            // Refund credits after failed generation
            if let Err(refund_err) = state
                .quota_service
                .refund_quota_weighted(identity.user_id, tier, operation)
                .await
            {
                tracing::error!(
                    user_id = %identity.user_id,
                    error = %refund_err,
                    "Failed to refund credits after text edit failure - user may have lost credits"
                );
            } else {
                tracing::info!(
                    user_id = %identity.user_id,
                    "Successfully refunded credits after text edit failure"
                );
            }
            Err(err)
        }
    }
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
        !node.content.is_empty() || node.summary.as_ref().is_some_and(|s| !s.is_empty())
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
    let generation_result = state
        .ai_service
        .generate_continuation_ideas(
            &request.story_context,
            &request.path_nodes,
            &request.generation_params,
            request.instructions.as_deref(),
            tier,
        )
        .await;

    // Handle errors with credit refund
    match generation_result {
        Ok(candidates) => Ok(Json(AITextContinueResponse { candidates })),
        Err(err) => {
            // Refund credits after failed generation
            if let Err(refund_err) = state
                .quota_service
                .refund_quota_weighted(identity.user_id, tier, AIOperation::ContinueIdeas)
                .await
            {
                tracing::error!(
                    user_id = %identity.user_id,
                    error = %refund_err,
                    "Failed to refund credits after ideas generation failure - user may have lost credits"
                );
            } else {
                tracing::info!(
                    user_id = %identity.user_id,
                    "Successfully refunded credits after ideas generation failure"
                );
            }
            Err(err)
        }
    }
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
    let generation_result = state
        .ai_service
        .generate_summaries(request.story_context.as_ref(), &request.nodes, tier)
        .await;

    // Handle errors with credit refund
    match generation_result {
        Ok(summaries) => Ok(Json(AITextSummarizeResponse { summaries })),
        Err(err) => {
            // Refund credits after failed generation
            if let Err(refund_err) = state
                .quota_service
                .refund_quota_weighted(identity.user_id, tier, AIOperation::Summarize)
                .await
            {
                tracing::error!(
                    user_id = %identity.user_id,
                    error = %refund_err,
                    "Failed to refund credits after summarize failure - user may have lost credits"
                );
            } else {
                tracing::info!(
                    user_id = %identity.user_id,
                    "Successfully refunded credits after summarize failure"
                );
            }
            Err(err)
        }
    }
}
