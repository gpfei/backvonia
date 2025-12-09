use axum::{extract::State, Json};
use tracing::instrument;

use crate::{
    app_state::AppState,
    error::{AppJson, Result},
    middleware::UserIdentity,
    models::{
        auth::{
            AppleSignInRequest, AuthResponse, LogoutRequest, LogoutResponse, MeResponse,
            RefreshTokenRequest, RefreshTokenResponse, UserResponse,
        },
        common::MessageResponse,
    },
};

/// POST /api/v1/auth/login/apple
///
/// Authenticate with Apple Sign In
///
/// Request body:
/// ```json
/// {
///   "idToken": "eyJ...",
///   "fullName": "John Doe",  // optional
///   "deviceInfo": {          // optional
///     "platform": "ios",
///     "deviceId": "...",
///     "appVersion": "1.0.0"
///   }
/// }
/// ```
///
/// Response:
/// ```json
/// {
///   "accessToken": "eyJ...",
///   "refreshToken": "550e8400-...",
///   "expiresIn": 900,
///   "user": {
///     "userId": "...",
///     "email": "user@privaterelay.appleid.com",
///     "fullName": "John Doe",
///     "status": "active",
///     "accountTier": "free",
///     "createdAt": "2024-12-04T00:00:00Z"
///   },
///   "welcomeBonus": {
///     "granted": true,
///     "amount": 5
///   }
/// }
/// ```
///
/// Note: `welcomeBonus` is only present for new user sign-ins
#[instrument(skip(state, request))]
pub async fn apple_sign_in(
    State(state): State<AppState>,
    AppJson(request): AppJson<AppleSignInRequest>,
) -> Result<Json<AuthResponse>> {
    // Convert device_info if present
    let device_info = request.device_info.map(|d| d.into());

    // Authenticate with Apple Sign In
    let auth_tokens = state
        .auth_service
        .authenticate_with_apple(&request.id_token, request.full_name, device_info)
        .await?;

    Ok(Json(AuthResponse {
        access_token: auth_tokens.access_token,
        refresh_token: auth_tokens.refresh_token,
        expires_in: auth_tokens.expires_in,
        user: auth_tokens.user.into(),
        welcome_bonus: auth_tokens.welcome_bonus.map(|b| b.into()),
    }))
}

/// POST /api/v1/auth/refresh
///
/// Refresh access token using refresh token
///
/// Request body:
/// ```json
/// {
///   "refreshToken": "550e8400-..."
/// }
/// ```
///
/// Response:
/// ```json
/// {
///   "accessToken": "eyJ...",
///   "expiresIn": 900
/// }
/// ```
#[instrument(skip(state, request))]
pub async fn refresh_token(
    State(state): State<AppState>,
    AppJson(request): AppJson<RefreshTokenRequest>,
) -> Result<Json<RefreshTokenResponse>> {
    // Refresh access token
    let (access_token, expires_in) = state
        .auth_service
        .refresh_access_token(&request.refresh_token)
        .await?;

    Ok(Json(RefreshTokenResponse {
        access_token,
        expires_in,
    }))
}

/// POST /api/v1/auth/logout
///
/// Logout - revoke a specific refresh token
///
/// Request body:
/// ```json
/// {
///   "refreshToken": "550e8400-..."
/// }
/// ```
///
/// Response:
/// ```json
/// {
///   "message": "Logged out successfully"
/// }
/// ```
#[instrument(skip(state, request))]
pub async fn logout(
    State(state): State<AppState>,
    AppJson(request): AppJson<LogoutRequest>,
) -> Result<Json<LogoutResponse>> {
    // Revoke the refresh token
    state.auth_service.logout(&request.refresh_token).await?;

    Ok(Json(MessageResponse::new("Logged out successfully")))
}

/// POST /api/v1/auth/logout-all
///
/// Logout from all devices - revoke all refresh tokens for the authenticated user
///
/// Requires: Authorization header with valid access token
///
/// Response:
/// ```json
/// {
///   "message": "Logged out from 3 devices"
/// }
/// ```
#[instrument(skip(state))]
pub async fn logout_all(
    State(state): State<AppState>,
    identity: UserIdentity,
) -> Result<Json<LogoutResponse>> {
    // Revoke all refresh tokens for user
    let revoked_count = state.auth_service.logout_all(identity.user_id).await?;

    Ok(Json(MessageResponse::new(format!(
        "Logged out from {} device(s)",
        revoked_count
    ))))
}

/// GET /api/v1/auth/me
///
/// Get current user information
///
/// Requires: Authorization header with valid access token
///
/// Response:
/// ```json
/// {
///   "userId": "...",
///   "email": "user@privaterelay.appleid.com",
///   "fullName": "John Doe",
///   "status": "active",
///   "accountTier": "pro",
///   "createdAt": "2024-12-04T00:00:00Z"
/// }
/// ```
#[instrument(skip(state))]
pub async fn get_me(
    State(state): State<AppState>,
    identity: UserIdentity,
) -> Result<Json<MeResponse>> {
    // Get user info
    let user_info = state.auth_service.get_user(identity.user_id).await?;

    Ok(Json(UserResponse::from(user_info)))
}
