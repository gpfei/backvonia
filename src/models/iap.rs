use entity::sea_orm_active_enums::AccountTier;
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::common::{IAPPlatform, PurchaseTier};

/// Internal structure for IAP verification result
#[derive(Debug, Clone)]
pub struct IAPVerification {
    pub purchase_identity: String,
    pub purchase_tier: PurchaseTier,
    pub product_id: Option<String>,
    pub valid_until: Option<time::OffsetDateTime>,
    pub is_family_shared: bool,
    pub subscription_status: Option<String>, // "active", "expired", "grace_period", etc.
}

// =============================================================================
// IAP Link (New User System)
// =============================================================================

/// IAP Link Request - Link receipt to authenticated user
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct IAPLinkRequest {
    pub platform: IAPPlatform,
    #[validate(length(min = 10, max = 100000))]
    pub receipt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IAPLinkResponse {
    pub account_tier: AccountTier,
    pub product_id: Option<String>,
    pub valid_until: Option<time::OffsetDateTime>,
}
