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
        let (original_transaction_id, product_id, expires_date_ms) =
            if let Some(transactions) = &apple_response.latest_receipt_info {
                let transaction = transactions
                    .first()
                    .ok_or_else(|| ApiError::InvalidReceipt("No transaction found".to_string()))?;

                (
                    transaction.original_transaction_id.clone(),
                    Some(transaction.product_id.clone()),
                    transaction.expires_date_ms.clone(),
                )
            } else if let Some(receipt) = &apple_response.receipt {
                (
                    receipt.original_transaction_id.clone(),
                    receipt.product_id.clone(),
                    None,
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
            "Successfully verified Apple IAP receipt: tier={:?}, product_id={:?}",
            purchase_tier, product_id
        );

        Ok(IAPVerification {
            purchase_identity: original_transaction_id,
            purchase_tier,
            product_id,
            valid_until,
            platform: IAPPlatform::Apple,
        })
    }

    /// Verify Google IAP receipt
    async fn verify_google_receipt(&self, receipt: &str) -> Result<IAPVerification> {
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
