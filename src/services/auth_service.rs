use crate::{
    config::AuthConfig,
    error::{ApiError, Result},
    services::{
        jwt_service::JWTService,
        refresh_token_service::{DeviceInfo, RefreshTokenService},
        welcome_bonus_service::WelcomeBonusService,
    },
};
use entity::{
    sea_orm_active_enums::{AccountTier, UserStatus},
    user_auth_methods, users,
};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation, Algorithm};
use sea_orm::{entity::*, query::*, sea_query::Expr, ActiveValue::Set, DatabaseConnection};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tracing::{info, warn, debug};
use uuid::Uuid;

/// Apple's JWKS endpoint
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
/// Apple's issuer
const APPLE_ISSUER: &str = "https://appleid.apple.com";
/// Cache duration for Apple's JWKS (1 hour)
const JWKS_CACHE_DURATION_SECS: i64 = 3600;

/// Apple ID token payload (subset of claims we care about)
#[derive(Debug, Deserialize)]
pub struct AppleIdTokenPayload {
    /// Subject - unique user identifier from Apple
    pub sub: String,
    /// Email (may be hidden/relay)
    pub email: Option<String>,
    /// Email verified flag
    pub email_verified: Option<bool>,
    /// Issuer (should be https://appleid.apple.com)
    pub iss: String,
    /// Audience (should be our app's client ID)
    pub aud: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
}

/// Cached JWKS with timestamp
struct CachedJwks {
    jwks: JwkSet,
    fetched_at: OffsetDateTime,
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

/// Welcome bonus information
#[derive(Debug, Serialize, Clone)]
pub struct WelcomeBonusInfo {
    pub granted: bool,
    pub amount: i32,
}

/// Authentication response with both tokens
#[derive(Debug, Serialize)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,  // Access token expiration in seconds
    pub user: UserInfo,
    pub welcome_bonus: Option<WelcomeBonusInfo>,
}

pub struct AuthService {
    db: DatabaseConnection,
    jwt_service: Arc<JWTService>,
    refresh_token_service: Arc<RefreshTokenService>,
    welcome_bonus_service: Arc<WelcomeBonusService>,
    config: Arc<AuthConfig>,
    /// Cached Apple JWKS for signature verification
    apple_jwks_cache: Arc<RwLock<Option<CachedJwks>>>,
}

impl AuthService {
    pub fn new(
        db: DatabaseConnection,
        jwt_service: Arc<JWTService>,
        refresh_token_service: Arc<RefreshTokenService>,
        welcome_bonus_service: Arc<WelcomeBonusService>,
        config: Arc<AuthConfig>,
    ) -> Self {
        Self {
            db,
            jwt_service,
            refresh_token_service,
            welcome_bonus_service,
            config,
            apple_jwks_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Get access token expiration in seconds from config
    fn access_token_expiration_seconds(&self) -> u64 {
        self.config.access_token_expiration_minutes * 60
    }

    /// Verify Apple ID token and find or create user
    ///
    /// This method:
    /// 1. Verifies the Apple ID token (validates JWT signature with Apple's JWKS)
    /// 2. Extracts the 'sub' (Apple's unique user ID)
    /// 3. Finds or creates user and auth_method records
    /// 4. Grants welcome bonus if eligible (new user + device_id provided)
    /// 5. Generates access token and refresh token
    /// 6. Returns AuthTokens with both tokens, user info, and welcome bonus status
    pub async fn authenticate_with_apple(
        &self,
        id_token: &str,
        full_name: Option<String>,
        device_info: Option<DeviceInfo>,
    ) -> Result<AuthTokens> {
        // Verify Apple ID token with full signature validation
        let apple_payload = self.verify_apple_id_token(id_token).await?;

        // Find or create user
        let (user, is_new_user) = self
            .find_or_create_user_by_apple(&apple_payload, full_name)
            .await?;

        // Check and grant welcome bonus for new users
        let mut welcome_bonus = None;
        if is_new_user {
            if let Some(ref device) = device_info {
                // Check eligibility
                let is_eligible = self
                    .welcome_bonus_service
                    .check_eligibility(&device.device_id, "apple", &apple_payload.sub)
                    .await?;

                if is_eligible {
                    // Grant bonus
                    let bonus_amount = self.config.welcome_bonus_amount;
                    match self
                        .welcome_bonus_service
                        .grant_bonus(
                            user.id,
                            &device.device_id,
                            "apple",
                            &apple_payload.sub,
                            bonus_amount,
                        )
                        .await
                    {
                        Ok(_) => {
                            info!(
                                user_id = %user.id,
                                amount = bonus_amount,
                                "Welcome bonus granted successfully"
                            );
                            welcome_bonus = Some(WelcomeBonusInfo {
                                granted: true,
                                amount: bonus_amount,
                            });
                        }
                        Err(e) => {
                            warn!(
                                user_id = %user.id,
                                error = %e,
                                "Failed to grant welcome bonus"
                            );
                            // Continue with auth flow even if bonus fails
                            welcome_bonus = Some(WelcomeBonusInfo {
                                granted: false,
                                amount: 0,
                            });
                        }
                    }
                } else {
                    info!(
                        user_id = %user.id,
                        "Welcome bonus not granted: eligibility check failed"
                    );
                    welcome_bonus = Some(WelcomeBonusInfo {
                        granted: false,
                        amount: 0,
                    });
                }
            } else {
                info!(
                    user_id = %user.id,
                    "Welcome bonus not granted: device_id not provided"
                );
                welcome_bonus = Some(WelcomeBonusInfo {
                    granted: false,
                    amount: 0,
                });
            }
        }

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

        // Return tokens + user info + welcome bonus status
        let user_info = UserInfo {
            user_id: user.id,
            email: user.email.clone(),
            full_name: user.full_name.clone(),
            status: user.status.clone(),
            account_tier: user.account_tier.clone(),
            created_at: user.created_at,
        };

        let expires_in = self.access_token_expiration_seconds();

        Ok(AuthTokens {
            access_token,
            refresh_token,
            expires_in,
            user: user_info,
            welcome_bonus,
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

        Ok((access_token, self.access_token_expiration_seconds()))
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
        let user_id = Uuid::now_v7(); // Use UUID v7 for time-ordered IDs (better for DB indexing)

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
            id: Set(Uuid::now_v7()), // Use UUID v7 for better indexing
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

    /// Verify and parse Apple ID token with full signature validation
    ///
    /// This method:
    /// 1. Fetches Apple's JWKS (with caching)
    /// 2. Extracts the key ID from the token header
    /// 3. Finds the matching public key
    /// 4. Verifies the JWT signature
    /// 5. Validates issuer, audience, and expiration
    /// 6. Returns the parsed payload
    async fn verify_apple_id_token(&self, id_token: &str) -> Result<AppleIdTokenPayload> {
        // 1. Get the token header to find the key ID (kid)
        let header = decode_header(id_token)
            .map_err(|e| ApiError::InvalidToken(format!("Invalid JWT header: {}", e)))?;

        let kid = header.kid
            .ok_or_else(|| ApiError::InvalidToken("Missing key ID in token header".to_string()))?;

        // 2. Get Apple's JWKS (cached)
        let jwks = self.get_apple_jwks().await?;

        // 3. Find the matching key
        let jwk = jwks.keys.iter()
            .find(|k| k.common.key_id.as_ref() == Some(&kid))
            .ok_or_else(|| ApiError::InvalidToken(format!("Key ID '{}' not found in Apple's JWKS", kid)))?;

        // 4. Create decoding key from JWK
        let decoding_key = DecodingKey::from_jwk(jwk)
            .map_err(|e| ApiError::InvalidToken(format!("Failed to create decoding key: {}", e)))?;

        // 5. Set up validation
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[APPLE_ISSUER]);
        validation.set_audience(&[&self.config.apple_client_id]);

        // 6. Decode and validate
        let token_data = decode::<AppleIdTokenPayload>(id_token, &decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    ApiError::ExpiredToken
                }
                jsonwebtoken::errors::ErrorKind::InvalidIssuer => {
                    ApiError::InvalidToken("Invalid token issuer".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                    ApiError::InvalidToken("Invalid token audience".to_string())
                }
                _ => ApiError::InvalidToken(format!("Token validation failed: {}", e)),
            })?;

        debug!(
            sub = %token_data.claims.sub,
            "Apple ID token verified successfully"
        );

        Ok(token_data.claims)
    }

    /// Fetch Apple's JWKS with caching
    async fn get_apple_jwks(&self) -> Result<JwkSet> {
        let now = OffsetDateTime::now_utc();

        // Check cache first
        {
            let cache = self.apple_jwks_cache.read().await;
            if let Some(ref cached) = *cache {
                let age = (now - cached.fetched_at).whole_seconds();
                if age < JWKS_CACHE_DURATION_SECS {
                    debug!("Using cached Apple JWKS (age: {}s)", age);
                    return Ok(cached.jwks.clone());
                }
            }
        }

        // Fetch fresh JWKS
        debug!("Fetching Apple JWKS from {}", APPLE_JWKS_URL);
        let response = reqwest::get(APPLE_JWKS_URL)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Failed to fetch Apple JWKS: {}", e)))?;

        if !response.status().is_success() {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "Apple JWKS request failed with status: {}",
                response.status()
            )));
        }

        let jwks: JwkSet = response
            .json()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Failed to parse Apple JWKS: {}", e)))?;

        // Update cache
        {
            let mut cache = self.apple_jwks_cache.write().await;
            *cache = Some(CachedJwks {
                jwks: jwks.clone(),
                fetched_at: now,
            });
        }

        info!("Apple JWKS fetched and cached ({} keys)", jwks.keys.len());
        Ok(jwks)
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
