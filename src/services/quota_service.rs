use crate::{
    config::QuotaConfig,
    error::{ApiError, Result},
    models::common::AIOperation,
};
use entity::sea_orm_active_enums::AccountTier;
use sea_orm::{
    entity::*, query::*, sea_query::OnConflict, DatabaseConnection, DatabaseTransaction,
    TransactionTrait,
};
use tracing::{info, instrument};
use uuid::Uuid;

pub struct QuotaService {
    db: DatabaseConnection,
    config: QuotaConfig,
}

impl QuotaService {
    pub fn new(db: DatabaseConnection, config: &QuotaConfig) -> Self {
        Self {
            db,
            config: config.clone(),
        }
    }

    /// Get monthly allocation for a tier
    fn get_monthly_allocation(&self, tier: &AccountTier) -> i32 {
        match tier {
            AccountTier::Free => self.config.free_text_daily_limit, // Treat as monthly for now
            AccountTier::Pro => self.config.pro_text_daily_limit,   // Treat as monthly for now
        }
    }

    /// Check and increment quota atomically with weighted cost
    /// Deducts from subscription first, then extra credits (FIFO-like behavior)
    #[instrument(skip(self))]
    pub async fn check_and_increment_quota_weighted(
        &self,
        user_id: Uuid,
        tier: &AccountTier,
        operation: AIOperation,
    ) -> Result<()> {
        let cost = operation.cost() as i32;
        let today = time::OffsetDateTime::now_utc().date();

        let txn = self.db.begin().await?;

        // 1. Lock credit balance (persistent across days)
        let balance = self
            .find_and_lock_credit_balance(user_id, tier, &txn)
            .await?;

        // 2. Check subscription reset
        let balance = self
            .reset_subscription_if_needed_tx(balance, tier, &txn)
            .await?;

        // 3. Calculate total available
        let total_available = balance.subscription_credits + balance.extra_credits_remaining;

        if total_available < cost {
            txn.rollback().await?;
            return Err(ApiError::QuotaExceeded(format!(
                "Insufficient credits: need {}, have {} (subscription: {}, extra: {})",
                cost,
                total_available,
                balance.subscription_credits,
                balance.extra_credits_remaining
            )));
        }

        // 4. Deduct from subscription first, then extra (subscription credits are "use it or lose it")
        let mut balance_active: entity::user_credit_balance::ActiveModel = balance.into();

        if *balance_active.subscription_credits.as_ref() >= cost {
            // Deduct entirely from subscription
            let current = *balance_active.subscription_credits.as_ref();
            balance_active.subscription_credits = Set(current - cost);
        } else {
            // Deduct partially from both
            let from_subscription = *balance_active.subscription_credits.as_ref();
            let from_extra = cost - from_subscription;

            balance_active.subscription_credits = Set(0);

            let current_extra = *balance_active.extra_credits_remaining.as_ref();
            balance_active.extra_credits_remaining = Set(current_extra - from_extra);
        }

        balance_active.last_updated = Set(time::OffsetDateTime::now_utc());
        let updated_balance = balance_active.update(&txn).await?;

        // 5. Update daily consumption log (for analytics)
        let usage = self.find_and_lock_usage(user_id, today, &txn).await?;
        let mut usage_active: entity::quota_usage::ActiveModel = usage.into();

        let is_image_op = matches!(operation, AIOperation::ImageGenerate);
        if is_image_op {
            let current = *usage_active.image_count.as_ref();
            usage_active.image_count = Set(current + cost);
        } else {
            let current = *usage_active.text_count.as_ref();
            usage_active.text_count = Set(current + cost);
        }
        usage_active.updated_at = Set(time::OffsetDateTime::now_utc());
        usage_active.update(&txn).await?;

        txn.commit().await?;

        // 6. Return updated status
        let total_remaining =
            updated_balance.subscription_credits + updated_balance.extra_credits_remaining;

        info!(
            "Deducted {} credits for {} operation by user: {} (remaining: {})",
            cost,
            if is_image_op { "image" } else { "text" },
            user_id,
            total_remaining
        );

        Ok(())
    }

    /// Refund credits after a failed operation
    /// Reverses the deduction made by check_and_increment_quota_weighted
    #[instrument(skip(self))]
    pub async fn refund_quota_weighted(
        &self,
        user_id: Uuid,
        tier: &AccountTier,
        operation: AIOperation,
    ) -> Result<()> {
        let cost = operation.cost() as i32;
        let today = time::OffsetDateTime::now_utc().date();

        let txn = self.db.begin().await?;

        // 1. Lock credit balance
        let balance = self
            .find_and_lock_credit_balance(user_id, tier, &txn)
            .await?;

        // 2. Refund credits (add them back)
        // Logic: Refund to extra credits first (they were deducted last in FIFO)
        // This is a simplification - we add to extra credits for safety
        let mut balance_active: entity::user_credit_balance::ActiveModel = balance.into();

        let current_extra = *balance_active.extra_credits_remaining.as_ref();
        balance_active.extra_credits_remaining = Set(current_extra + cost);
        balance_active.last_updated = Set(time::OffsetDateTime::now_utc());
        balance_active.update(&txn).await?;

        // 3. Decrement daily usage log (for analytics accuracy)
        let usage = self.find_and_lock_usage(user_id, today, &txn).await?;
        let mut usage_active: entity::quota_usage::ActiveModel = usage.into();

        let is_image_op = matches!(operation, AIOperation::ImageGenerate);
        if is_image_op {
            let current = *usage_active.image_count.as_ref();
            // Prevent negative counts
            usage_active.image_count = Set(std::cmp::max(0, current - cost));
        } else {
            let current = *usage_active.text_count.as_ref();
            usage_active.text_count = Set(std::cmp::max(0, current - cost));
        }
        usage_active.updated_at = Set(time::OffsetDateTime::now_utc());
        usage_active.update(&txn).await?;

        txn.commit().await?;

        info!(
            "Refunded {} credits for {} operation to user: {} (new extra credits: {})",
            cost,
            if is_image_op { "image" } else { "text" },
            user_id,
            current_extra + cost
        );

        Ok(())
    }

    /// Helper: Find and lock credit balance for update (within transaction)
    /// IMPORTANT: When creating, syncs extra credits from credits_events
    async fn find_and_lock_credit_balance(
        &self,
        user_id: Uuid,
        tier: &AccountTier,
        txn: &DatabaseTransaction,
    ) -> Result<entity::user_credit_balance::Model> {
        // Try to find with lock
        let balance = entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::UserId.eq(user_id))
            .lock_exclusive()
            .one(txn)
            .await?;

        if let Some(balance) = balance {
            return Ok(balance);
        }

        // Calculate extra credits from ledger events
        // This handles the case where user received credits BEFORE first quota check
        let events = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::UserId.eq(user_id))
            .filter(entity::credits_events::Column::RevokedAt.is_null())
            .filter(entity::credits_events::Column::EventType.ne("consumption"))
            .all(txn)
            .await?;

        let extra_credits: i32 = events.iter().map(|p| p.amount - p.consumed).sum();

        // If not found, insert (no-op if another transaction races) then re-lock
        let now = time::OffsetDateTime::now_utc();
        let next_month = now + time::Duration::days(30);
        let monthly_allocation = self.get_monthly_allocation(tier);

        let new_balance = entity::user_credit_balance::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            subscription_credits: Set(monthly_allocation),
            subscription_monthly_allocation: Set(monthly_allocation),
            subscription_resets_at: Set(Some(next_month)),
            extra_credits_remaining: Set(extra_credits), // Sync from purchases!
            last_updated: Set(now),
            created_at: Set(now),
        };

        entity::user_credit_balance::Entity::insert(new_balance)
            .on_conflict(
                OnConflict::column(entity::user_credit_balance::Column::UserId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(txn)
            .await?;

        entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::UserId.eq(user_id))
            .lock_exclusive()
            .one(txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to create or lock credit balance record"
                ))
            })
    }

    /// Helper: Reset subscription credits if needed (within transaction)
    async fn reset_subscription_if_needed_tx(
        &self,
        balance: entity::user_credit_balance::Model,
        tier: &AccountTier,
        txn: &DatabaseTransaction,
    ) -> Result<entity::user_credit_balance::Model> {
        let now = time::OffsetDateTime::now_utc();

        // Check if reset is needed
        if let Some(resets_at) = balance.subscription_resets_at {
            if now >= resets_at {
                // Reset subscription credits
                let monthly_allocation = self.get_monthly_allocation(tier);
                let next_reset = now + time::Duration::days(30);

                let mut balance_active: entity::user_credit_balance::ActiveModel = balance.into();
                balance_active.subscription_credits = Set(monthly_allocation);
                balance_active.subscription_resets_at = Set(Some(next_reset));
                balance_active.last_updated = Set(now);

                let updated = balance_active.update(txn).await?;

                info!(
                    "Reset subscription credits for user: {} to {}",
                    updated.user_id, monthly_allocation
                );

                return Ok(updated);
            }
        }

        Ok(balance)
    }

    /// Helper: Find and lock usage record for update
    async fn find_and_lock_usage(
        &self,
        user_id: Uuid,
        date: time::Date,
        txn: &DatabaseTransaction,
    ) -> Result<entity::quota_usage::Model> {
        // Try to find with lock
        let usage = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::UserId.eq(user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(date))
            .lock_exclusive()
            .one(txn)
            .await?;

        if let Some(usage) = usage {
            return Ok(usage);
        }

        // If not found, insert (no-op if another transaction races) then re-lock.
        let now = time::OffsetDateTime::now_utc();

        let new_usage = entity::quota_usage::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            usage_date: Set(date),
            text_count: Set(0),
            image_count: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            extra_credits_total: Set(0),
            subscription_credits: Set(0),
            subscription_monthly_allocation: Set(0),
            last_extra_credits_sync: NotSet,
            subscription_resets_at: NotSet,
        };

        entity::quota_usage::Entity::insert(new_usage)
            .on_conflict(
                OnConflict::columns([
                    entity::quota_usage::Column::UserId,
                    entity::quota_usage::Column::UsageDate,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(txn)
            .await?;

        entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::UserId.eq(user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(date))
            .lock_exclusive()
            .one(txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to create or lock quota usage record"
                ))
            })
    }
}
