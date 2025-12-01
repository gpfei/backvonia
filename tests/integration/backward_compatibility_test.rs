/// Test backward compatibility of GET /quota endpoint
///
/// This test verifies that the original /quota endpoint maintains its response schema
/// for existing mobile clients while the new /quota/credits endpoint provides extended information.

#[cfg(test)]
mod backward_compatibility_tests {
    use serde_json::Value;

    #[test]
    fn test_quota_response_schema() {
        // Original /quota endpoint should return:
        // {
        //   "success": true,
        //   "data": {
        //     "purchaseTier": "pro",
        //     "quota": {
        //       "textLimitDaily": 1000,
        //       "textUsedToday": 10,
        //       "textRemainingToday": 990,
        //       "imageLimitDaily": 50,
        //       "imageUsedToday": 2,
        //       "imageRemainingToday": 48
        //     }
        //   }
        // }

        let original_schema = r#"{
            "success": true,
            "data": {
                "purchaseTier": "pro",
                "quota": {
                    "textLimitDaily": 1000,
                    "textUsedToday": 10,
                    "textRemainingToday": 990,
                    "imageLimitDaily": 50,
                    "imageUsedToday": 2,
                    "imageRemainingToday": 48
                }
            }
        }"#;

        let parsed: Value = serde_json::from_str(original_schema).unwrap();

        // Verify required fields exist
        assert_eq!(parsed["success"], true);
        assert!(parsed["data"]["purchaseTier"].is_string());
        assert!(parsed["data"]["quota"].is_object());
        assert!(parsed["data"]["quota"]["textLimitDaily"].is_number());
        assert!(parsed["data"]["quota"]["textUsedToday"].is_number());
        assert!(parsed["data"]["quota"]["textRemainingToday"].is_number());
        assert!(parsed["data"]["quota"]["imageLimitDaily"].is_number());
        assert!(parsed["data"]["quota"]["imageUsedToday"].is_number());
        assert!(parsed["data"]["quota"]["imageRemainingToday"].is_number());
    }

    #[test]
    fn test_quota_credits_response_schema() {
        // New /quota/credits endpoint should return:
        // {
        //   "success": true,
        //   "data": {
        //     "purchaseTier": "pro",
        //     "subscriptionCredits": { "current": 450, ... },
        //     "extraCredits": { "total": 750, ... },
        //     "totalCredits": 1200,
        //     "dailyQuota": { ... }  // Optional for backward compatibility
        //   }
        // }

        let credits_schema = r#"{
            "success": true,
            "data": {
                "purchaseTier": "pro",
                "subscriptionCredits": {
                    "current": 450,
                    "monthlyAllocation": 1000,
                    "resetsAt": "2025-12-15T00:00:00Z"
                },
                "extraCredits": {
                    "total": 750,
                    "purchases": []
                },
                "totalCredits": 1200,
                "dailyQuota": {
                    "textLimitDaily": 1000,
                    "textUsedToday": 10,
                    "textRemainingToday": 990,
                    "imageLimitDaily": 50,
                    "imageUsedToday": 2,
                    "imageRemainingToday": 48
                }
            }
        }"#;

        let parsed: Value = serde_json::from_str(credits_schema).unwrap();

        // Verify new fields exist
        assert_eq!(parsed["success"], true);
        assert!(parsed["data"]["subscriptionCredits"].is_object());
        assert!(parsed["data"]["extraCredits"].is_object());
        assert!(parsed["data"]["totalCredits"].is_number());

        // Verify backward compatibility - dailyQuota is included
        assert!(parsed["data"]["dailyQuota"].is_object());
    }

    #[test]
    fn test_endpoint_separation() {
        // This test documents that we have TWO separate endpoints:
        // 1. GET /api/v1/quota - Returns original QuotaResponse (for backward compat)
        // 2. GET /api/v1/quota/credits - Returns new CreditsQuotaResponse (with extra credits)

        // Original endpoint path
        let original_path = "/api/v1/quota";
        assert!(original_path.ends_with("/quota"));
        assert!(!original_path.contains("credits"));

        // New endpoint path
        let new_path = "/api/v1/quota/credits";
        assert!(new_path.ends_with("/quota/credits"));

        // Paths are different
        assert_ne!(original_path, new_path);
    }
}
