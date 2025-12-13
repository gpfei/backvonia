use crate::models::common::ErrorResponse;
use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

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

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("Token expired")]
    ExpiredToken,

    #[error("User not found: {0}")]
    UserNotFound(String),

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
                    StatusCode::FAILED_DEPENDENCY,
                    "AI_PROVIDER_ERROR",
                    "AI service temporarily unavailable".to_string(),
                )
            }
            ApiError::BadRequest(ref msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            ApiError::NotFound(ref msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            ApiError::Unauthorized(ref msg) => {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone())
            }
            ApiError::InvalidToken(ref msg) => {
                (StatusCode::UNAUTHORIZED, "INVALID_TOKEN", msg.clone())
            }
            ApiError::ExpiredToken => (
                StatusCode::UNAUTHORIZED,
                "EXPIRED_TOKEN",
                "Token has expired".to_string(),
            ),
            ApiError::UserNotFound(ref msg) => {
                (StatusCode::NOT_FOUND, "USER_NOT_FOUND", msg.clone())
            }
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

        let body = ErrorResponse::new(error_code, message, None);

        (status, Json(body)).into_response()
    }
}

// Helper type for results
pub type Result<T> = std::result::Result<T, ApiError>;

/// Custom JSON extractor that returns proper JSON error responses on deserialization failures
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> std::result::Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => {
                let message = match rejection {
                    JsonRejection::JsonDataError(err) => {
                        format!(
                            "Failed to deserialize the JSON body into the target type: {}",
                            err
                        )
                    }
                    JsonRejection::JsonSyntaxError(err) => {
                        format!("Invalid JSON syntax: {}", err)
                    }
                    JsonRejection::MissingJsonContentType(err) => {
                        format!("Missing JSON Content-Type header: {}", err)
                    }
                    JsonRejection::BytesRejection(err) => {
                        format!("Failed to read request body: {}", err)
                    }
                    _ => format!("Invalid JSON request: {}", rejection),
                };
                Err(ApiError::BadRequest(message))
            }
        }
    }
}
