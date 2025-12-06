use crate::models::common::MessageResponse;
use entity::sea_orm_active_enums::{AccountTier, UserStatus};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

// ============================================================================
// Request Models
// ============================================================================

/// Request body for Apple Sign In
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleSignInRequest {
    /// Apple ID token (JWT from Apple Sign In)
    pub id_token: String,
    /// User's full name (optional, only provided on first sign in)
    pub full_name: Option<String>,
    /// Device information for tracking sessions
    pub device_info: Option<DeviceInfoRequest>,
}

/// Device information for session tracking
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfoRequest {
    pub platform: String,            // ios, ipados, macos
    pub device_id: String,           // X-Device-Id header
    pub app_version: Option<String>, // X-Client-Version header
}

/// Request body for refreshing access token
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// Request body for logout
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutRequest {
    pub refresh_token: String,
}

// ============================================================================
// Response Models (HTTP 2xx payloads)
// ============================================================================

/// Authentication data containing tokens and user info
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64, // Access token expiration in seconds
    pub user: UserResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_bonus: Option<WelcomeBonusResponse>,
}

/// User information in responses
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    pub user_id: Uuid,
    pub email: Option<String>,
    pub full_name: Option<String>,
    pub status: UserStatus,
    pub account_tier: AccountTier,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Welcome bonus information in auth response
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeBonusResponse {
    pub granted: bool,
    pub amount: i32,
}

/// Response from token refresh
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenResponse {
    pub access_token: String,
    pub expires_in: u64, // Access token expiration in seconds
}

/// Response from logout (message-only response)
pub type LogoutResponse = MessageResponse;

/// Response from /auth/me endpoint
pub type MeResponse = UserResponse;

// ============================================================================
// Conversion Functions
// ============================================================================

impl From<crate::services::refresh_token_service::DeviceInfo> for DeviceInfoRequest {
    fn from(device_info: crate::services::refresh_token_service::DeviceInfo) -> Self {
        Self {
            platform: device_info.platform,
            device_id: device_info.device_id,
            app_version: device_info.app_version,
        }
    }
}

impl From<DeviceInfoRequest> for crate::services::refresh_token_service::DeviceInfo {
    fn from(request: DeviceInfoRequest) -> Self {
        Self {
            platform: request.platform,
            device_id: request.device_id,
            app_version: request.app_version,
        }
    }
}

impl From<crate::services::auth_service::UserInfo> for UserResponse {
    fn from(user_info: crate::services::auth_service::UserInfo) -> Self {
        Self {
            user_id: user_info.user_id,
            email: user_info.email,
            full_name: user_info.full_name,
            status: user_info.status,
            account_tier: user_info.account_tier,
            created_at: user_info.created_at,
        }
    }
}

impl From<crate::services::auth_service::WelcomeBonusInfo> for WelcomeBonusResponse {
    fn from(bonus_info: crate::services::auth_service::WelcomeBonusInfo) -> Self {
        Self {
            granted: bonus_info.granted,
            amount: bonus_info.amount,
        }
    }
}
