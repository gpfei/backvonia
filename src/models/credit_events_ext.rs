/// Extension methods for credits_events entity
///
/// This file contains custom business logic methods that complement
/// the auto-generated entity in entity/src/credits_events.rs
use entity::credits_events;

/// Extension trait for CreditEvent model
pub trait CreditEventExt {
    /// Calculate remaining credits (amount - consumed)
    fn remaining(&self) -> i32;

    /// Check if there are remaining credits
    fn has_remaining(&self) -> bool;
}

impl CreditEventExt for credits_events::Model {
    fn remaining(&self) -> i32 {
        self.amount - self.consumed
    }

    fn has_remaining(&self) -> bool {
        self.remaining() > 0
    }
}
