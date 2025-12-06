//! Rate limiting middleware using Redis
//!
//! Implements token bucket rate limiting with Redis backend.
//! Different limits apply based on account tier and endpoint.

use crate::{
    error::{ApiError, Result},
    middleware::jwt_auth::UserIdentity,
};
use axum::{extract::Request, middleware::Next, response::Response};
use entity::sea_orm_active_enums::AccountTier;
use redis::{AsyncCommands, Client};
use std::sync::Arc;
use tracing::{debug, warn};

/// Rate limit configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests per minute for free tier
    pub free_tier_rpm: u32,
    /// Requests per minute for pro tier
    pub pro_tier_rpm: u32,
    /// Window size in seconds
    pub window_seconds: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            free_tier_rpm: 60, // 1 request per second
            pro_tier_rpm: 600, // 10 requests per second
            window_seconds: 60,
        }
    }
}

/// Rate limiting middleware
///
/// Uses sliding window counter in Redis to track request rates per identity.
/// Returns 429 Too Many Requests when limit is exceeded.
pub fn rate_limit_middleware(
    redis_client: Arc<Client>,
    config: RateLimitConfig,
) -> impl Fn(
    Request,
    Next,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response>> + Send>>
       + Clone {
    move |request: Request, next: Next| {
        let redis_client = redis_client.clone();
        let config = config.clone();

        Box::pin(async move {
            // Extract identity from request extensions (set by auth middleware)
            let identity = request.extensions().get::<UserIdentity>().ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Rate limit middleware requires jwt_auth_middleware"
                ))
            })?;

            // Determine rate limit based on tier
            let limit = match identity.account_tier {
                AccountTier::Free => config.free_tier_rpm,
                AccountTier::Pro => config.pro_tier_rpm,
            };

            // Check rate limit using Redis (using user_id as the key)
            let allowed = check_rate_limit(
                &redis_client,
                &identity.user_id.to_string(),
                limit,
                config.window_seconds,
            )
            .await?;

            if !allowed {
                warn!(
                    "Rate limit exceeded for user: {} (tier: {:?})",
                    identity.user_id, identity.account_tier
                );
                return Err(ApiError::RateLimitExceeded);
            }

            debug!(
                "Rate limit check passed for user: {} (tier: {:?})",
                identity.user_id, identity.account_tier
            );

            // Continue to next middleware/handler
            Ok(next.run(request).await)
        })
    }
}

/// Check rate limit using Redis sliding window counter
///
/// Returns true if request is allowed, false if rate limit exceeded.
async fn check_rate_limit(
    redis_client: &Client,
    user_id: &str,
    limit: u32,
    window_seconds: u32,
) -> Result<bool> {
    let mut conn = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Redis connection failed: {}", e)))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let key = format!("rate_limit:user:{}", user_id);
    let window_start = now - window_seconds as u64;

    // Use Redis sorted set with timestamps as scores
    // Remove old entries outside the window
    let _: () = conn
        .zrembyscore(&key, 0, window_start as f64)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Redis ZREMRANGEBYSCORE failed: {}", e)))?;

    // Count requests in current window
    let count: u32 = conn
        .zcard(&key)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Redis ZCARD failed: {}", e)))?;

    // Check if under limit
    if count >= limit {
        return Ok(false);
    }

    // Add current request to sorted set
    let member = format!("{}:{}", now, uuid::Uuid::new_v4());
    let _: () = conn
        .zadd(&key, member, now as f64)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Redis ZADD failed: {}", e)))?;

    // Set expiration on key (window + buffer)
    let _: () = conn
        .expire(&key, (window_seconds + 10) as i64)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Redis EXPIRE failed: {}", e)))?;

    Ok(true)
}

/// Create rate limit middleware with default configuration
pub fn create_rate_limiter(
    redis_client: Arc<Client>,
) -> impl Fn(
    Request,
    Next,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response>> + Send>>
       + Clone {
    rate_limit_middleware(redis_client, RateLimitConfig::default())
}

/// Create rate limit middleware with custom configuration
pub fn create_rate_limiter_with_config(
    redis_client: Arc<Client>,
    config: RateLimitConfig,
) -> impl Fn(
    Request,
    Next,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response>> + Send>>
       + Clone {
    rate_limit_middleware(redis_client, config)
}
