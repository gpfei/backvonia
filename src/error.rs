use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("Invalid IAP receipt: {0}")]
    InvalidReceipt(String),

    #[error("AI provider error: {0}")]
    AIProvider(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            ApiError::Database(ref e) => {
                tracing::error!("Database error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DATABASE_ERROR",
                    "An internal database error occurred".to_string(),
                )
            }
            ApiError::QuotaExceeded(ref msg) => {
                (StatusCode::TOO_MANY_REQUESTS, "QUOTA_EXCEEDED", msg.clone())
            }
            ApiError::InvalidReceipt(ref msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_RECEIPT",
                msg.clone(),
            ),
            ApiError::AIProvider(ref msg) => {
                tracing::error!("AI provider error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    "AI_PROVIDER_ERROR",
                    "AI service temporarily unavailable".to_string(),
                )
            }
            ApiError::BadRequest(ref msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            ApiError::NotFound(ref msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            ApiError::Unauthorized(ref msg) => {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone())
            }
            ApiError::Conflict(ref msg) => (StatusCode::CONFLICT, "CONFLICT", msg.clone()),
            ApiError::RateLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMIT_EXCEEDED",
                "Too many requests, please try again later".to_string(),
            ),
            ApiError::Internal(ref e) => {
                tracing::error!("Internal error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "An internal error occurred".to_string(),
                )
            }
        };

        let body = json!({
            "success": false,
            "error": {
                "code": error_code,
                "message": message,
            }
        });

        (status, Json(body)).into_response()
    }
}

// Helper type for results
pub type Result<T> = std::result::Result<T, ApiError>;
