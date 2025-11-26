use backvonia::middleware::IAPIdentity;
use backvonia::models::common::PurchaseTier;

#[test]
fn test_iap_identity_struct() {
    // Basic test to verify IAPIdentity structure works
    let identity = IAPIdentity {
        purchase_identity: "test_user_123".to_string(),
        purchase_tier: PurchaseTier::Free,
    };

    assert_eq!(identity.purchase_identity, "test_user_123");
    assert_eq!(identity.purchase_tier, PurchaseTier::Free);
}

#[test]
fn test_purchase_tier_variants() {
    let free = PurchaseTier::Free;
    let pro = PurchaseTier::Pro;

    assert_ne!(free, pro);
}
