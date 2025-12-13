use crate::{
    config::AuthConfig,
    error::{ApiError, Result},
};
use entity::refresh_tokens;
use sea_orm::{entity::*, query::*, sea_query::Expr, ActiveValue::Set, DatabaseConnection};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// Device information stored with refresh token
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub platform: String,            // ios, ipados, macos
    pub device_id: String,           // X-Device-Id header
    pub app_version: Option<String>, // X-Client-Version header
}

pub struct RefreshTokenService {
    db: DatabaseConnection,
    config: Arc<AuthConfig>,
}

impl RefreshTokenService {
    pub fn new(db: DatabaseConnection, config: Arc<AuthConfig>) -> Self {
        Self { db, config }
    }

    /// Generate and store a new refresh token
    pub async fn create_refresh_token(
        &self,
        user_id: Uuid,
        device_info: Option<DeviceInfo>,
    ) -> Result<String> {
        // Generate opaque token (UUID)
        let refresh_token = Uuid::new_v4().to_string();

        // Hash token before storing (never store plaintext tokens)
        let token_hash = Self::hash_token(&refresh_token);

        // Calculate expiration
        let now = OffsetDateTime::now_utc();
        let expires_at =
            now + time::Duration::days(self.config.refresh_token_expiration_days as i64);

        // Store in database
        let new_token = refresh_tokens::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            token_hash: Set(token_hash),
            expires_at: Set(expires_at),
            created_at: Set(now),
            last_used_at: Set(None),
            revoked_at: Set(None),
            device_info: Set(device_info.map(|d| json!(d))),
        };

        refresh_tokens::Entity::insert(new_token)
            .exec(&self.db)
            .await?;

        Ok(refresh_token)
    }

    /// Validate refresh token and return user_id
    ///
    /// This method:
    /// 1. Hashes the provided token
    /// 2. Looks up the token in the database
    /// 3. Checks if it's expired or revoked
    /// 4. Updates last_used_at
    /// 5. Returns the user_id
    pub async fn validate_and_update_refresh_token(&self, refresh_token: &str) -> Result<Uuid> {
        let token_hash = Self::hash_token(refresh_token);
        let now = OffsetDateTime::now_utc();

        // Find token by hash
        let token_record = refresh_tokens::Entity::find()
            .filter(refresh_tokens::Column::TokenHash.eq(&token_hash))
            .one(&self.db)
            .await?
            .ok_or_else(|| ApiError::InvalidToken("Refresh token not found".to_string()))?;

        // Check if revoked
        if token_record.revoked_at.is_some() {
            return Err(ApiError::InvalidToken(
                "Refresh token has been revoked".to_string(),
            ));
        }

        // Check if expired
        if token_record.expires_at < now {
            return Err(ApiError::ExpiredToken);
        }

        // Update last_used_at
        refresh_tokens::Entity::update_many()
            .filter(refresh_tokens::Column::Id.eq(token_record.id))
            .col_expr(refresh_tokens::Column::LastUsedAt, Expr::value(Some(now)))
            .exec(&self.db)
            .await?;

        Ok(token_record.user_id)
    }

    /// Revoke a specific refresh token
    pub async fn revoke_refresh_token(&self, refresh_token: &str) -> Result<()> {
        let token_hash = Self::hash_token(refresh_token);
        let now = OffsetDateTime::now_utc();

        let result = refresh_tokens::Entity::update_many()
            .filter(refresh_tokens::Column::TokenHash.eq(&token_hash))
            .col_expr(refresh_tokens::Column::RevokedAt, Expr::value(Some(now)))
            .exec(&self.db)
            .await?;

        if result.rows_affected == 0 {
            return Err(ApiError::InvalidToken(
                "Refresh token not found".to_string(),
            ));
        }

        Ok(())
    }

    /// Revoke all refresh tokens for a user (logout from all devices)
    pub async fn revoke_all_user_tokens(&self, user_id: Uuid) -> Result<u64> {
        let now = OffsetDateTime::now_utc();

        let result = refresh_tokens::Entity::update_many()
            .filter(refresh_tokens::Column::UserId.eq(user_id))
            .filter(refresh_tokens::Column::RevokedAt.is_null())
            .col_expr(refresh_tokens::Column::RevokedAt, Expr::value(Some(now)))
            .exec(&self.db)
            .await?;

        Ok(result.rows_affected)
    }

    /// Hash token using SHA256 (for secure storage)
    fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_token() {
        let token = "550e8400-e29b-41d4-a716-446655440000";
        let hash1 = RefreshTokenService::hash_token(token);
        let hash2 = RefreshTokenService::hash_token(token);

        // Same token should produce same hash
        assert_eq!(hash1, hash2);

        // Hash should be 64 chars (SHA256 hex)
        assert_eq!(hash1.len(), 64);

        // Different token should produce different hash
        let different_token = "different-token";
        let hash3 = RefreshTokenService::hash_token(different_token);
        assert_ne!(hash1, hash3);
    }
}
