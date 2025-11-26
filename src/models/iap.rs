use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::common::{IAPPlatform, PurchaseTier, Quota};

/// IAP Verify Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct IAPVerifyRequest {
    pub platform: IAPPlatform,
    #[validate(length(min = 10, max = 100000))]
    pub receipt: String,
    #[validate(length(max = 50))]
    pub app_version: Option<String>,
    #[validate(length(max = 100))]
    pub device_id: Option<String>,
}

/// IAP Verify Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IAPVerifyResponse {
    pub success: bool,
    pub data: IAPVerifyData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IAPVerifyData {
    pub purchase_tier: PurchaseTier,
    pub purchase_identity: String,
    pub product_id: Option<String>,
    pub valid_until: Option<DateTime<Utc>>,
    pub platform: IAPPlatform,
}

/// Quota Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaResponse {
    pub success: bool,
    pub data: QuotaData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaData {
    pub purchase_tier: PurchaseTier,
    pub quota: Quota,
}

/// Internal structure for IAP verification result
#[derive(Debug, Clone)]
pub struct IAPVerification {
    pub purchase_identity: String,
    pub purchase_tier: PurchaseTier,
    pub product_id: Option<String>,
    pub valid_until: Option<DateTime<Utc>>,
    pub platform: IAPPlatform,
}
