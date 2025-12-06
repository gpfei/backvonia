use entity::sea_orm_active_enums::AccountTier;
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::common::{IAPPlatform, SuccessResponse};

/// Request to record a credit purchase
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct CreditPurchaseRequest {
    #[validate(length(min = 1, max = 255))]
    pub transaction_id: String,

    #[validate(length(max = 255))]
    pub original_transaction_id: Option<String>,

    #[validate(length(min = 1, max = 100))]
    pub product_id: String,

    pub platform: IAPPlatform,

    pub purchase_date: time::OffsetDateTime,

    #[validate(length(max = 100000))]
    pub receipt: Option<String>,
}

/// Response for credit purchase recording
pub type CreditPurchaseResponse = SuccessResponse<CreditPurchaseData>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPurchaseData {
    pub credits_added: i32,
    pub total_extra_credits: i32,
    pub purchase_id: uuid::Uuid,
    pub quota: CreditsQuotaInfo,
}

/// Single credit purchase record
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreditPurchaseRecord {
    pub transaction_id: String,
    pub product_id: String,
    pub amount: i32,
    pub consumed: i32,
    pub remaining: i32,
    pub purchase_date: time::OffsetDateTime,
}

/// Subscription credits information
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionCreditsInfo {
    pub current: i32,
    pub monthly_allocation: i32,
    pub resets_at: Option<time::OffsetDateTime>,
}

/// Extra credits information
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExtraCreditsInfo {
    pub total: i32,
    pub purchases: Vec<CreditPurchaseRecord>,
}

/// Complete quota information including credits
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreditsQuotaInfo {
    pub subscription_credits: SubscriptionCreditsInfo,
    pub extra_credits: ExtraCreditsInfo,
    pub total_credits: i32,
}

/// Updated quota response with credit information
pub type CreditsQuotaResponse = SuccessResponse<CreditsQuotaData>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditsQuotaData {
    pub account_tier: AccountTier,
    pub subscription_credits: SubscriptionCreditsInfo,
    pub extra_credits: ExtraCreditsInfo,
    pub total_credits: i32,
}

/// Error response for duplicate transaction
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTransactionError {
    pub code: String,
    pub message: String,
    pub details: DuplicateTransactionDetails,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTransactionDetails {
    pub transaction_id: String,
    pub previously_granted_at: time::OffsetDateTime,
}

impl CreditPurchaseRequest {
    /// Extract credit amount from product ID
    pub fn extract_credit_amount(&self) -> Option<i32> {
        match self.product_id.as_str() {
            "com.talevonia.tale.credits.100" => Some(100),
            "com.talevonia.tale.credits.500" => Some(500),
            "com.talevonia.tale.credits.2000" => Some(2000),
            _ => None,
        }
    }
}
