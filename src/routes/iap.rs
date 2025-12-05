use axum::{extract::State, Json};
use entity::sea_orm_active_enums::AccountTier;
use tracing::{info, instrument};
use validator::Validate;

use crate::{
    app_state::AppState,
    error::{ApiError, Result},
    middleware::UserIdentity,
    models::iap::{IAPLinkData, IAPLinkRequest, IAPLinkResponse},
};

/// POST /api/v1/iap/verify
///
/// Verify IAP receipt and link to authenticated user account.
/// Updates user's account_tier based on the receipt.
///
/// **Requires authentication** - user must be logged in first.
///
/// Flow:
/// 1. Verifies receipt with Apple/Google
/// 2. Links purchase to user account
/// 3. Updates user's account_tier (Free/Pro)
/// 4. Returns updated account status
#[instrument(skip(state, request))]
pub async fn verify_iap(
    State(state): State<AppState>,
    identity: UserIdentity,
    Json(request): Json<IAPLinkRequest>,
) -> Result<Json<IAPLinkResponse>> {
    use entity::{user_iap_receipts, users};
    use sea_orm::{ActiveModelTrait, EntityTrait, Set, TransactionTrait};

    // Validate request
    request
        .validate()
        .map_err(|e| ApiError::BadRequest(format!("Validation error: {}", e)))?;

    // Verify the receipt
    let verification = state
        .iap_service
        .verify_receipt(request.platform, &request.receipt)
        .await?;

    // Determine account tier based on receipt
    // If has active subscription (Pro tier purchase), set to Pro, otherwise Free
    let account_tier = match verification.purchase_tier {
        crate::models::common::PurchaseTier::Pro => AccountTier::Pro,
        crate::models::common::PurchaseTier::Free => AccountTier::Free,
    };

    // Hash the receipt for storage
    let receipt_hash = state.iap_service.hash_receipt(&request.receipt);
    let now = time::OffsetDateTime::now_utc();

    // Use transaction to atomically:
    // 1. Store receipt in user_iap_receipts
    // 2. Update user's account_tier
    let txn = state.db.begin().await?;

    // Store receipt in user_iap_receipts table
    let receipt_id = uuid::Uuid::now_v7();
    let platform_str = match request.platform {
        crate::models::common::IAPPlatform::Apple => "apple",
        crate::models::common::IAPPlatform::Google => "google",
    };

    let new_receipt = user_iap_receipts::ActiveModel {
        id: Set(receipt_id),
        user_id: Set(identity.user_id),
        original_transaction_id: Set(verification.purchase_identity.clone()),
        platform: Set(platform_str.to_string()),
        is_family_shared: Set(verification.is_family_shared),
        family_primary_user_id: Set(None), // TODO: Implement family primary user detection
        product_id: Set(verification.product_id.clone().unwrap_or_default()),
        purchase_tier: Set(account_tier.clone()),
        subscription_status: Set(verification.subscription_status.clone()),
        expires_at: Set(verification.valid_until),
        receipt_hash: Set(receipt_hash),
        last_verified_at: Set(now),
        first_linked_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
    };

    // Insert or update receipt (handle duplicate original_transaction_id + user_id)
    let receipt_result = user_iap_receipts::Entity::insert(new_receipt)
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([
                user_iap_receipts::Column::UserId,
                user_iap_receipts::Column::OriginalTransactionId,
            ])
            .update_columns([
                user_iap_receipts::Column::ReceiptHash,
                user_iap_receipts::Column::LastVerifiedAt,
                user_iap_receipts::Column::PurchaseTier,
                user_iap_receipts::Column::ExpiresAt,
                user_iap_receipts::Column::SubscriptionStatus,
                user_iap_receipts::Column::IsFamilyShared,
                user_iap_receipts::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec(&txn)
        .await;

    if let Err(e) = receipt_result {
        tracing::warn!(
            user_id = %identity.user_id,
            error = %e,
            "Failed to store IAP receipt"
        );
        return Err(ApiError::from(e));
    }

    // Update user's account_tier
    let user = users::Entity::find_by_id(identity.user_id)
        .one(&txn)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    let mut user_active: users::ActiveModel = user.into();
    user_active.account_tier = Set(account_tier.clone());
    user_active.updated_at = Set(now);

    user_active.update(&txn).await?;

    // Commit transaction
    txn.commit().await?;

    info!(
        user_id = %identity.user_id,
        account_tier = ?account_tier,
        product_id = ?verification.product_id,
        original_transaction_id = %verification.purchase_identity,
        "IAP linked to user account and tier updated"
    );

    Ok(Json(IAPLinkResponse {
        success: true,
        data: IAPLinkData {
            account_tier,
            product_id: verification.product_id,
            valid_until: verification.valid_until,
        },
    }))
}
