use crate::{
    error::{ApiError, Result},
    services::{jwt_service::JWTService, refresh_token_service::{DeviceInfo, RefreshTokenService}},
};
use entity::{
    sea_orm_active_enums::{AccountTier, UserStatus},
    user_auth_methods, users,
};
use sea_orm::{
    entity::*, query::*, sea_query::Expr, ActiveValue::Set, DatabaseConnection,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// Apple ID token payload (subset of claims we care about)
#[derive(Debug, Deserialize)]
pub struct AppleIdTokenPayload {
    /// Subject - unique user identifier from Apple
    pub sub: String,
    /// Email (may be hidden/relay)
    pub email: Option<String>,
    /// Email verified flag
    pub email_verified: Option<bool>,
}

/// User info returned after authentication
#[derive(Debug, Serialize, Clone)]
pub struct UserInfo {
    pub user_id: Uuid,
    pub email: Option<String>,
    pub full_name: Option<String>,
    pub status: UserStatus,
    pub account_tier: AccountTier,
    pub created_at: OffsetDateTime,
}

/// Authentication response with both tokens
#[derive(Debug, Serialize)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,  // Access token expiration in seconds
    pub user: UserInfo,
}

pub struct AuthService {
    db: DatabaseConnection,
    jwt_service: Arc<JWTService>,
    refresh_token_service: Arc<RefreshTokenService>,
}

impl AuthService {
    pub fn new(
        db: DatabaseConnection,
        jwt_service: Arc<JWTService>,
        refresh_token_service: Arc<RefreshTokenService>,
    ) -> Self {
        Self {
            db,
            jwt_service,
            refresh_token_service,
        }
    }

    /// Verify Apple ID token and find or create user
    ///
    /// This method:
    /// 1. Verifies the Apple ID token (in production, would validate JWT signature with Apple's JWKS)
    /// 2. Extracts the 'sub' (Apple's unique user ID)
    /// 3. Finds or creates user and auth_method records
    /// 4. Generates access token (15 min) and refresh token (7 days)
    /// 5. Returns AuthTokens with both tokens and user info
    pub async fn authenticate_with_apple(
        &self,
        id_token: &str,
        full_name: Option<String>,
        device_info: Option<DeviceInfo>,
    ) -> Result<AuthTokens> {
        // Parse and verify Apple ID token
        // NOTE: In production, this should:
        // 1. Fetch Apple's JWKS from https://appleid.apple.com/auth/keys
        // 2. Verify JWT signature with public key
        // 3. Validate issuer, audience, expiration
        //
        // For MVP, we'll do basic JWT parsing
        let apple_payload = self.parse_apple_id_token(id_token)?;

        // Find or create user
        let (user, _is_new_user) = self
            .find_or_create_user_by_apple(&apple_payload, full_name)
            .await?;

        // Generate access token (short-lived, 15 min)
        let access_token = self
            .jwt_service
            .generate_token(user.id, user.account_tier.clone())?;

        // Generate refresh token (long-lived, 7 days)
        let refresh_token = self
            .refresh_token_service
            .create_refresh_token(user.id, device_info)
            .await?;

        // Update last_login_at
        self.update_last_login(user.id).await?;

        // Return tokens + user info
        let user_info = UserInfo {
            user_id: user.id,
            email: user.email.clone(),
            full_name: user.full_name.clone(),
            status: user.status.clone(),
            account_tier: user.account_tier.clone(),
            created_at: user.created_at,
        };

        Ok(AuthTokens {
            access_token,
            refresh_token,
            expires_in: 900,  // 15 minutes in seconds
            user: user_info,
        })
    }

    /// Refresh access token using refresh token
    ///
    /// This method:
    /// 1. Validates the refresh token (checks DB, expiration, revocation)
    /// 2. Extracts the user_id from refresh token
    /// 3. Fetches current user data (to get latest account_tier)
    /// 4. Generates a new access token
    /// 5. Returns new access token
    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<(String, u64)> {
        // Validate refresh token and get user_id
        let user_id = self
            .refresh_token_service
            .validate_and_update_refresh_token(refresh_token)
            .await?;

        // Get current user data (for latest account_tier)
        let user = users::Entity::find_by_id(user_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| ApiError::UserNotFound(user_id.to_string()))?;

        // Generate new access token
        let access_token = self
            .jwt_service
            .generate_token(user.id, user.account_tier)?;

        Ok((access_token, 900)) // 15 minutes in seconds
    }

    /// Logout - revoke a specific refresh token
    pub async fn logout(&self, refresh_token: &str) -> Result<()> {
        self.refresh_token_service
            .revoke_refresh_token(refresh_token)
            .await
    }

    /// Logout from all devices - revoke all refresh tokens for user
    pub async fn logout_all(&self, user_id: Uuid) -> Result<u64> {
        self.refresh_token_service
            .revoke_all_user_tokens(user_id)
            .await
    }

    /// Find or create user by Apple Sign In
    async fn find_or_create_user_by_apple(
        &self,
        apple_payload: &AppleIdTokenPayload,
        full_name: Option<String>,
    ) -> Result<(users::Model, bool)> {
        // Look for existing auth_method with this provider_user_id
        let existing_auth = user_auth_methods::Entity::find()
            .filter(user_auth_methods::Column::Provider.eq("apple"))
            .filter(user_auth_methods::Column::ProviderUserId.eq(&apple_payload.sub))
            .one(&self.db)
            .await?;

        if let Some(auth_method) = existing_auth {
            // User exists - fetch user record
            let user = users::Entity::find_by_id(auth_method.user_id)
                .one(&self.db)
                .await?
                .ok_or_else(|| ApiError::UserNotFound(auth_method.user_id.to_string()))?;

            return Ok((user, false));
        }

        // New user - create user + auth_method records
        let now = OffsetDateTime::now_utc();
        let user_id = Uuid::new_v4();

        // Create user
        let new_user = users::ActiveModel {
            id: Set(user_id),
            email: Set(apple_payload.email.clone()),
            email_verified: Set(apple_payload.email_verified.unwrap_or(false)),
            full_name: Set(full_name),
            status: Set(UserStatus::Active),
            account_tier: Set(AccountTier::Free),
            created_at: Set(now),
            updated_at: Set(now),
            last_login_at: Set(Some(now)),
        };

        let user = users::Entity::insert(new_user)
            .exec_with_returning(&self.db)
            .await?;

        // Create auth_method
        let new_auth_method = user_auth_methods::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            provider: Set("apple".to_string()),
            provider_user_id: Set(apple_payload.sub.clone()),
            provider_email: Set(apple_payload.email.clone()),
            provider_metadata: Set(None),
            first_linked_at: Set(now),
            last_used_at: Set(now),
        };

        user_auth_methods::Entity::insert(new_auth_method)
            .exec(&self.db)
            .await?;

        Ok((user, true))
    }

    /// Update user's last_login_at timestamp
    async fn update_last_login(&self, user_id: Uuid) -> Result<()> {
        let now = OffsetDateTime::now_utc();

        users::Entity::update_many()
            .filter(users::Column::Id.eq(user_id))
            .col_expr(users::Column::LastLoginAt, Expr::value(Some(now)))
            .col_expr(users::Column::UpdatedAt, Expr::value(now))
            .exec(&self.db)
            .await?;

        Ok(())
    }

    /// Parse Apple ID token (basic JWT parsing without signature verification)
    ///
    /// NOTE: In production, this MUST verify the JWT signature with Apple's public keys
    /// from https://appleid.apple.com/auth/keys
    fn parse_apple_id_token(&self, id_token: &str) -> Result<AppleIdTokenPayload> {
        // Split JWT parts
        let parts: Vec<&str> = id_token.split('.').collect();
        if parts.len() != 3 {
            return Err(ApiError::InvalidToken("Invalid JWT format".to_string()));
        }

        // Decode payload (base64url)
        use base64::{Engine as _, engine::general_purpose};
        let payload_bytes = general_purpose::URL_SAFE_NO_PAD.decode(parts[1])
            .map_err(|e| ApiError::InvalidToken(format!("Failed to decode payload: {}", e)))?;

        let payload: AppleIdTokenPayload = serde_json::from_slice(&payload_bytes)
            .map_err(|e| ApiError::InvalidToken(format!("Failed to parse payload: {}", e)))?;

        Ok(payload)
    }

    /// Get user by ID
    pub async fn get_user(&self, user_id: Uuid) -> Result<UserInfo> {
        let user = users::Entity::find_by_id(user_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| ApiError::UserNotFound(user_id.to_string()))?;

        Ok(UserInfo {
            user_id: user.id,
            email: user.email.clone(),
            full_name: user.full_name.clone(),
            status: user.status.clone(),
            account_tier: user.account_tier.clone(),
            created_at: user.created_at,
        })
    }

    /// Update user's account tier (admin operation)
    pub async fn update_account_tier(
        &self,
        user_id: Uuid,
        new_tier: AccountTier,
    ) -> Result<UserInfo> {
        let now = OffsetDateTime::now_utc();

        users::Entity::update_many()
            .filter(users::Column::Id.eq(user_id))
            .col_expr(users::Column::AccountTier, Expr::value(new_tier.clone()))
            .col_expr(users::Column::UpdatedAt, Expr::value(now))
            .exec(&self.db)
            .await?;

        self.get_user(user_id).await
    }
}
