use serde::{Deserialize, Serialize};

/// Generic success response wrapper
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessResponse<T> {
    pub success: bool,
    pub data: T,
}

impl<T> SuccessResponse<T> {
    pub fn new(data: T) -> Self {
        Self {
            success: true,
            data,
        }
    }
}

/// Error response structure
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub success: bool,
    pub error: ErrorObject,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorObject {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Purchase tier enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PurchaseTier {
    Free,
    Pro,
}

impl PurchaseTier {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "free" => Some(Self::Free),
            "pro" => Some(Self::Pro),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pro => "pro",
        }
    }
}

/// IAP Platform
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IAPPlatform {
    Apple,
    Google,
}

impl IAPPlatform {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "apple" => Some(Self::Apple),
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }
}

/// Quota information (full details)
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    // Subscription credits (monthly allocation)
    pub subscription_credits: i32,
    pub subscription_monthly_allocation: i32,
    pub subscription_resets_at: Option<String>, // ISO 8601 timestamp

    // Extra purchased credits
    pub extra_credits_total: i32,
    pub extra_credits_consumed: i32,
    pub extra_credits_remaining: i32,

    // Total available credits
    pub total_credits_remaining: i32,
}

/// Subset of quota info for AI API responses
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSubset {
    // Total remaining credits (usable for any operation: text or image)
    pub credits_remaining: i32,
}

/// AI Operation types with weighted quota costs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIOperation {
    ContinueProse,
    ContinueIdeas,
    EditExpand,
    EditShorten,
    EditRewrite,
    EditFixGrammar,
    ImageGenerate,
}

impl AIOperation {
    /// Get the quota cost for this operation
    pub fn cost(&self) -> u32 {
        match self {
            AIOperation::ContinueProse => 5,
            AIOperation::ContinueIdeas => 3,
            AIOperation::EditExpand => 2,
            AIOperation::EditShorten => 2,
            AIOperation::EditRewrite => 2,
            AIOperation::EditFixGrammar => 1,
            AIOperation::ImageGenerate => 10, // Images are more expensive
        }
    }
}
