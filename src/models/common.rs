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

/// Quota information
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub text_limit_daily: i32,
    pub text_used_today: i32,
    pub text_remaining_today: i32,
    pub image_limit_daily: i32,
    pub image_used_today: i32,
    pub image_remaining_today: i32,
}

/// Subset of quota info for responses
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSubset {
    pub text_remaining_today: i32,
    pub image_remaining_today: i32,
}
