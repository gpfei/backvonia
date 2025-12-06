use crate::{config::AuthConfig, error::Result};
use entity::sea_orm_active_enums::AccountTier;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// JWT claims structure
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject (user_id)
    pub sub: String,
    /// Account tier
    pub tier: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expiration (Unix timestamp)
    pub exp: i64,
}

pub struct JWTService {
    config: Arc<AuthConfig>,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JWTService {
    pub fn new(config: Arc<AuthConfig>) -> Self {
        let encoding_key = EncodingKey::from_secret(config.jwt_secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(config.jwt_secret.as_bytes());

        Self {
            config,
            encoding_key,
            decoding_key,
        }
    }

    /// Generate a JWT access token for a user (short-lived)
    pub fn generate_token(&self, user_id: Uuid, account_tier: AccountTier) -> Result<String> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let exp = now + (self.config.access_token_expiration_minutes as i64 * 60);

        let claims = Claims {
            sub: user_id.to_string(),
            tier: match account_tier {
                AccountTier::Free => "free".to_string(),
                AccountTier::Pro => "pro".to_string(),
            },
            iat: now,
            exp,
        };

        let token = encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| crate::error::ApiError::Internal(e.into()))?;

        Ok(token)
    }

    /// Validate and decode a JWT token
    pub fn validate_token(&self, token: &str) -> Result<Claims> {
        let token_data = decode::<Claims>(token, &self.decoding_key, &Validation::default())
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    crate::error::ApiError::ExpiredToken
                }
                _ => crate::error::ApiError::InvalidToken(e.to_string()),
            })?;

        Ok(token_data.claims)
    }

    /// Extract user_id from claims
    pub fn user_id_from_claims(claims: &Claims) -> Result<Uuid> {
        Uuid::parse_str(&claims.sub)
            .map_err(|e| crate::error::ApiError::InvalidToken(format!("Invalid user_id: {}", e)))
    }

    /// Extract account_tier from claims
    pub fn account_tier_from_claims(claims: &Claims) -> Result<AccountTier> {
        match claims.tier.as_str() {
            "free" => Ok(AccountTier::Free),
            "pro" => Ok(AccountTier::Pro),
            _ => Err(crate::error::ApiError::InvalidToken(format!(
                "Invalid account tier: {}",
                claims.tier
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Arc<AuthConfig> {
        Arc::new(AuthConfig {
            jwt_secret: "test-secret-key-with-minimum-32-characters-required".to_string(),
            access_token_expiration_minutes: 15,
            refresh_token_expiration_days: 7,
            apple_client_id: "com.test.app".to_string(),
            apple_team_id: "TEST123456".to_string(),
            welcome_bonus_amount: 5,
        })
    }

    #[test]
    fn test_generate_and_validate_token() {
        let service = JWTService::new(test_config());
        let user_id = Uuid::new_v4();
        let tier = AccountTier::Pro;

        // Generate token
        let token = service.generate_token(user_id, tier.clone()).unwrap();
        assert!(!token.is_empty());

        // Validate token
        let claims = service.validate_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.tier, "pro");

        // Extract user_id
        let extracted_user_id = JWTService::user_id_from_claims(&claims).unwrap();
        assert_eq!(extracted_user_id, user_id);

        // Extract tier
        let extracted_tier = JWTService::account_tier_from_claims(&claims).unwrap();
        assert_eq!(extracted_tier, AccountTier::Pro);
    }

    #[test]
    fn test_invalid_token() {
        let service = JWTService::new(test_config());
        let result = service.validate_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_all_tiers() {
        let service = JWTService::new(test_config());
        let user_id = Uuid::new_v4();

        for tier in [AccountTier::Free, AccountTier::Pro] {
            let token = service.generate_token(user_id, tier.clone()).unwrap();
            let claims = service.validate_token(&token).unwrap();
            let extracted_tier = JWTService::account_tier_from_claims(&claims).unwrap();
            assert_eq!(extracted_tier, tier);
        }
    }
}
