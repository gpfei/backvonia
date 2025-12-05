use crate::{
    config::IAPConfig,
    error::{ApiError, Result},
    models::{
        common::{IAPPlatform, PurchaseTier},
        iap::IAPVerification,
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, instrument, warn};

pub struct IAPService {
    config: IAPConfig,
    http_client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct AppleReceiptResponse {
    status: i32,
    receipt: Option<AppleReceipt>,
    latest_receipt_info: Option<Vec<AppleTransaction>>,
}

#[derive(Debug, Deserialize)]
struct AppleReceipt {
    original_transaction_id: String,
    product_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppleTransaction {
    original_transaction_id: String,
    product_id: String,
    expires_date_ms: Option<String>,
    #[serde(default)]
    in_app_ownership_type: Option<String>, // "PURCHASED" or "FAMILY_SHARED"
    #[serde(default)]
    cancellation_date_ms: Option<String>,
    #[serde(default)]
    is_in_billing_retry_period: Option<String>, // "true" or "false"
    #[serde(default)]
    is_in_intro_offer_period: Option<String>,
    #[serde(default)]
    is_trial_period: Option<String>,
}

impl IAPService {
    pub fn new(config: &IAPConfig) -> Self {
        Self {
            config: config.clone(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Verify IAP receipt and extract purchase information
    #[instrument(skip(self, receipt))]
    pub async fn verify_receipt(
        &self,
        platform: IAPPlatform,
        receipt: &str,
    ) -> Result<IAPVerification> {
        match platform {
            IAPPlatform::Apple => self.verify_apple_receipt(receipt).await,
            IAPPlatform::Google => self.verify_google_receipt(receipt).await,
        }
    }

    /// Verify Apple IAP receipt
    async fn verify_apple_receipt(&self, receipt: &str) -> Result<IAPVerification> {
        // Determine endpoint based on environment
        let endpoint = match self.config.apple_environment.as_str() {
            "production" => "https://buy.itunes.apple.com/verifyReceipt",
            _ => "https://sandbox.itunes.apple.com/verifyReceipt",
        };

        let request_body = serde_json::json!({
            "receipt-data": receipt,
            "password": self.config.apple_shared_secret,
            "exclude-old-transactions": true,
        });

        let response = self
            .http_client
            .post(endpoint)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ApiError::InvalidReceipt(format!("Failed to verify receipt: {}", e)))?;

        let apple_response: AppleReceiptResponse = response
            .json()
            .await
            .map_err(|e| ApiError::InvalidReceipt(format!("Invalid response format: {}", e)))?;

        // Check status code
        if apple_response.status != 0 {
            return Err(ApiError::InvalidReceipt(format!(
                "Invalid receipt status: {}",
                apple_response.status
            )));
        }

        // Extract transaction info from latest_receipt_info or receipt
        let transaction_opt = apple_response
            .latest_receipt_info
            .as_ref()
            .and_then(|txns| txns.first());

        let (original_transaction_id, product_id, expires_date_ms, is_family_shared, subscription_status) =
            if let Some(transaction) = transaction_opt {
                // Extract family sharing status
                let is_family_shared = transaction
                    .in_app_ownership_type
                    .as_deref()
                    .map(|t| t == "FAMILY_SHARED")
                    .unwrap_or(false);

                // Determine subscription status
                let subscription_status = Self::determine_subscription_status(
                    transaction.expires_date_ms.as_deref(),
                    transaction.cancellation_date_ms.as_deref(),
                    transaction.is_in_billing_retry_period.as_deref(),
                );

                (
                    transaction.original_transaction_id.clone(),
                    Some(transaction.product_id.clone()),
                    transaction.expires_date_ms.clone(),
                    is_family_shared,
                    subscription_status,
                )
            } else if let Some(receipt) = &apple_response.receipt {
                // No transaction info - likely a non-subscription purchase
                (
                    receipt.original_transaction_id.clone(),
                    receipt.product_id.clone(),
                    None,
                    false, // No family sharing info available
                    None,  // No subscription status for non-subscriptions
                )
            } else {
                return Err(ApiError::InvalidReceipt(
                    "No receipt or transaction found".to_string(),
                ));
            };

        // Determine tier based on product_id
        let purchase_tier = match product_id.as_deref() {
            Some("com.talevonia.pro.monthly") => PurchaseTier::Pro,
            Some("com.talevonia.pro.yearly") => PurchaseTier::Pro,
            Some("com.talevonia.pro") => PurchaseTier::Pro,
            Some(id) if id.contains("pro") => PurchaseTier::Pro, // Fallback for pro variants
            _ => PurchaseTier::Free,
        };

        // Parse expiration for subscriptions
        let valid_until = expires_date_ms
            .and_then(|ms| ms.parse::<i64>().ok())
            .and_then(|ts_ms| time::OffsetDateTime::from_unix_timestamp(ts_ms / 1000).ok());

        info!(
            "Successfully verified Apple IAP receipt: tier={:?}, product_id={:?}, family_shared={}, status={:?}",
            purchase_tier, product_id, is_family_shared, subscription_status
        );

        Ok(IAPVerification {
            purchase_identity: original_transaction_id,
            purchase_tier,
            product_id,
            valid_until,
            platform: IAPPlatform::Apple,
            is_family_shared,
            subscription_status,
        })
    }

    /// Determine subscription status from Apple receipt data
    fn determine_subscription_status(
        expires_date_ms: Option<&str>,
        cancellation_date_ms: Option<&str>,
        is_in_billing_retry: Option<&str>,
    ) -> Option<String> {
        let now = time::OffsetDateTime::now_utc();

        // If cancelled, status is "cancelled"
        if cancellation_date_ms.is_some() {
            return Some("cancelled".to_string());
        }

        // If in billing retry period, status is "billing_retry"
        if is_in_billing_retry == Some("true") {
            return Some("billing_retry".to_string());
        }

        // Check expiration date
        if let Some(expires_ms) = expires_date_ms {
            if let Ok(ts_ms) = expires_ms.parse::<i64>() {
                if let Ok(expires_date) = time::OffsetDateTime::from_unix_timestamp(ts_ms / 1000) {
                    if expires_date > now {
                        return Some("active".to_string());
                    } else {
                        // Check if in grace period (7 days after expiration)
                        let grace_period_end = expires_date + time::Duration::days(7);
                        if now < grace_period_end {
                            return Some("grace_period".to_string());
                        } else {
                            return Some("expired".to_string());
                        }
                    }
                }
            }
        }

        // No expiration date = likely not a subscription, or status unknown
        None
    }

    /// Verify Google IAP receipt
    async fn verify_google_receipt(&self, _receipt: &str) -> Result<IAPVerification> {
        // TODO: Implement Google Play verification
        // This would use Google Play Developer API with service account credentials
        // For MVP, return a placeholder or error

        warn!("Google IAP verification not yet implemented");

        Err(ApiError::InvalidReceipt(
            "Google IAP verification not yet implemented".to_string(),
        ))
    }

    /// Generate hash for receipt caching
    pub fn hash_receipt(&self, receipt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(receipt.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}
