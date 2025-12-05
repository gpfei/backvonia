use backvonia::middleware::UserIdentity;
use entity::sea_orm_active_enums::AccountTier;
use uuid::Uuid;

#[test]
fn test_user_identity_struct() {
    // Basic test to verify UserIdentity structure works
    let identity = UserIdentity {
        user_id: Uuid::new_v4(),
        account_tier: AccountTier::Free,
    };

    assert_eq!(identity.account_tier, AccountTier::Free);
}

#[test]
fn test_account_tier_variants() {
    let free = AccountTier::Free;
    let pro = AccountTier::Pro;

    assert_ne!(free, pro);
}
