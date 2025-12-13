/// Extension methods for credits_events entity
///
/// This file contains custom business logic methods that complement
/// the auto-generated entity in entity/src/credits_events.rs
use entity::credits_events;

/// Extension trait for CreditEvent model
pub trait CreditEventExt {
    /// Calculate remaining credits (amount - consumed)
    fn remaining(&self) -> i32;
}

impl CreditEventExt for credits_events::Model {
    fn remaining(&self) -> i32 {
        self.amount - self.consumed
    }
}
