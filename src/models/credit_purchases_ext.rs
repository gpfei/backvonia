/// Extension methods for credit_purchases entity
///
/// This file contains custom business logic methods that complement
/// the auto-generated entity in entity/src/credit_purchases.rs
use entity::credit_purchases;

/// Extension trait for CreditPurchase model
pub trait CreditPurchaseExt {
    /// Calculate remaining credits (amount - consumed)
    fn remaining(&self) -> i32;

    /// Check if this purchase has been revoked
    fn is_revoked(&self) -> bool;

    /// Check if there are remaining credits
    fn has_remaining(&self) -> bool;
}

impl CreditPurchaseExt for credit_purchases::Model {
    fn remaining(&self) -> i32 {
        self.amount - self.consumed
    }

    fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    fn has_remaining(&self) -> bool {
        self.remaining() > 0
    }
}
