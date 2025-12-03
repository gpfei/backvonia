use crate::{
    config::QuotaConfig,
    error::{ApiError, Result},
    models::common::{AIOperation, PurchaseTier, Quota, QuotaSubset},
};
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

#[derive(Debug, Clone)]
pub struct QuotaStatus {
    // Total credits available (subscription + extra)
    pub total_credits_remaining: i32,

    // Breakdown for detailed responses
    pub subscription_credits: i32,
    pub subscription_monthly_allocation: i32,
    pub subscription_resets_at: Option<time::OffsetDateTime>,
    pub extra_credits_remaining: i32,
}

impl QuotaService {
    pub fn new(db: DatabaseConnection, config: &QuotaConfig) -> Self {
        Self {
            db,
            config: config.clone(),
        }
    }

    /// Get monthly allocation for a tier
    fn get_monthly_allocation(&self, tier: PurchaseTier) -> i32 {
        match tier {
            PurchaseTier::Free => self.config.free_text_daily_limit, // Treat as monthly for now
            PurchaseTier::Pro => self.config.pro_text_daily_limit,   // Treat as monthly for now
        }
    }

    /// Check current quota status for a purchase identity
    /// Returns total available credits (subscription + extra purchases)
    #[instrument(skip(self))]
    pub async fn check_quota(&self, identity: &str, tier: PurchaseTier) -> Result<QuotaStatus> {
        // Get or create credit balance
        let balance = self.get_or_create_credit_balance(identity, tier).await?;

        // Check if subscription needs reset
        let balance = self.reset_subscription_if_needed(balance, tier).await?;

        // Calculate total remaining
        let total_remaining = balance.subscription_credits + balance.extra_credits_remaining;

        Ok(QuotaStatus {
            total_credits_remaining: total_remaining,
            subscription_credits: balance.subscription_credits,
            subscription_monthly_allocation: balance.subscription_monthly_allocation,
            subscription_resets_at: balance.subscription_resets_at,
            extra_credits_remaining: balance.extra_credits_remaining,
        })
    }

    /// Check and increment quota atomically with weighted cost
    /// Deducts from subscription first, then extra credits (FIFO-like behavior)
    #[instrument(skip(self))]
    pub async fn check_and_increment_quota_weighted(
        &self,
        identity: &str,
        tier: PurchaseTier,
        operation: AIOperation,
    ) -> Result<QuotaStatus> {
        let cost = operation.cost() as i32;
        let today = time::OffsetDateTime::now_utc().date();

        let txn = self.db.begin().await?;

        // 1. Lock credit balance (persistent across days)
        let balance = self
            .find_and_lock_credit_balance(identity, tier, &txn)
            .await?;

        // 2. Check subscription reset
        let balance = self.reset_subscription_if_needed_tx(balance, tier, &txn).await?;

        // 3. Calculate total available
        let total_available = balance.subscription_credits + balance.extra_credits_remaining;

        if total_available < cost {
            txn.rollback().await?;
            return Err(ApiError::QuotaExceeded(format!(
                "Insufficient credits: need {}, have {} (subscription: {}, extra: {})",
                cost, total_available, balance.subscription_credits, balance.extra_credits_remaining
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
        let usage = self.find_and_lock_usage(identity, today, &txn).await?;
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
            "Deducted {} credits for {} operation by identity: {} (remaining: {})",
            cost,
            if is_image_op { "image" } else { "text" },
            identity,
            total_remaining
        );

        Ok(QuotaStatus {
            total_credits_remaining: total_remaining,
            subscription_credits: updated_balance.subscription_credits,
            subscription_monthly_allocation: updated_balance.subscription_monthly_allocation,
            subscription_resets_at: updated_balance.subscription_resets_at,
            extra_credits_remaining: updated_balance.extra_credits_remaining,
        })
    }

    /// Get full quota info
    pub async fn get_quota_info(&self, identity: &str, tier: PurchaseTier) -> Result<Quota> {
        let status = self.check_quota(identity, tier).await?;

        // Query total purchased credits
        let total_purchased = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::LocalUserId.eq(identity))
            .filter(entity::credit_purchases::Column::RevokedAt.is_null())
            .select_only()
            .column_as(
                entity::credit_purchases::Column::Amount.sum(),
                "total_amount",
            )
            .into_tuple::<Option<i32>>()
            .one(&self.db)
            .await?
            .flatten()
            .unwrap_or(0);

        let extra_consumed = total_purchased - status.extra_credits_remaining;

        Ok(Quota {
            subscription_credits: status.subscription_credits,
            subscription_monthly_allocation: status.subscription_monthly_allocation,
            subscription_resets_at: status.subscription_resets_at.map(|dt| dt.to_string()),
            extra_credits_total: total_purchased,
            extra_credits_consumed: extra_consumed,
            extra_credits_remaining: status.extra_credits_remaining,
            total_credits_remaining: status.total_credits_remaining,
        })
    }

    /// Get quota subset for AI API responses (DEPRECATED - quota no longer returned in AI responses)
    pub async fn get_quota_subset(
        &self,
        identity: &str,
        tier: PurchaseTier,
    ) -> Result<QuotaSubset> {
        let status = self.check_quota(identity, tier).await?;

        Ok(QuotaSubset {
            credits_remaining: status.total_credits_remaining,
        })
    }

    /// Add extra credits from purchase
    #[instrument(skip(self))]
    pub async fn add_extra_credits(
        &self,
        identity: &str,
        tier: PurchaseTier,
        amount: i32,
    ) -> Result<()> {
        let txn = self.db.begin().await?;

        let balance = self
            .find_and_lock_credit_balance(identity, tier, &txn)
            .await?;
        let mut balance_active: entity::user_credit_balance::ActiveModel = balance.into();

        let current_extra = *balance_active.extra_credits_remaining.as_ref();
        balance_active.extra_credits_remaining = Set(current_extra + amount);
        balance_active.last_updated = Set(time::OffsetDateTime::now_utc());
        balance_active.update(&txn).await?;

        txn.commit().await?;

        info!(
            "Added {} extra credits to identity: {} (new total extra: {})",
            amount,
            identity,
            current_extra + amount
        );

        Ok(())
    }

    /// Helper: Get or create credit balance for a user
    /// IMPORTANT: When creating, syncs extra credits from credit_purchases
    async fn get_or_create_credit_balance(
        &self,
        identity: &str,
        tier: PurchaseTier,
    ) -> Result<entity::user_credit_balance::Model> {
        // Try to find existing balance
        if let Some(balance) = entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::PurchaseIdentity.eq(identity))
            .one(&self.db)
            .await?
        {
            return Ok(balance);
        }

        // Calculate extra credits from purchases
        // This handles the case where user bought credits BEFORE first quota check
        let purchases = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::LocalUserId.eq(identity))
            .filter(entity::credit_purchases::Column::RevokedAt.is_null())
            .all(&self.db)
            .await?;

        let extra_credits: i32 = purchases
            .iter()
            .map(|p| p.amount - p.consumed)
            .sum();

        // Create initial balance on first request
        let now = time::OffsetDateTime::now_utc();
        let next_month = now + time::Duration::days(30);
        let monthly_allocation = self.get_monthly_allocation(tier);

        let new_balance = entity::user_credit_balance::ActiveModel {
            id: Set(Uuid::new_v4()),
            purchase_identity: Set(identity.to_string()),
            subscription_credits: Set(monthly_allocation),
            subscription_monthly_allocation: Set(monthly_allocation),
            subscription_resets_at: Set(Some(next_month)),
            extra_credits_remaining: Set(extra_credits), // Sync from purchases!
            last_updated: Set(now),
            created_at: Set(now),
        };

        // Insert with ON CONFLICT DO NOTHING (race condition safety)
        entity::user_credit_balance::Entity::insert(new_balance)
            .on_conflict(
                OnConflict::column(entity::user_credit_balance::Column::PurchaseIdentity)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        // Return the existing or newly-inserted row
        entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::PurchaseIdentity.eq(identity))
            .one(&self.db)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to find credit balance record after upsert"
                ))
            })
    }

    /// Helper: Find and lock credit balance for update (within transaction)
    /// IMPORTANT: When creating, syncs extra credits from credit_purchases
    async fn find_and_lock_credit_balance(
        &self,
        identity: &str,
        tier: PurchaseTier,
        txn: &DatabaseTransaction,
    ) -> Result<entity::user_credit_balance::Model> {
        // Try to find with lock
        let balance = entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::PurchaseIdentity.eq(identity))
            .lock_exclusive()
            .one(txn)
            .await?;

        if let Some(balance) = balance {
            return Ok(balance);
        }

        // Calculate extra credits from purchases
        // This handles the case where user bought credits BEFORE first quota check
        let purchases = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::LocalUserId.eq(identity))
            .filter(entity::credit_purchases::Column::RevokedAt.is_null())
            .all(txn)
            .await?;

        let extra_credits: i32 = purchases
            .iter()
            .map(|p| p.amount - p.consumed)
            .sum();

        // If not found, insert (no-op if another transaction races) then re-lock
        let now = time::OffsetDateTime::now_utc();
        let next_month = now + time::Duration::days(30);
        let monthly_allocation = self.get_monthly_allocation(tier);

        let new_balance = entity::user_credit_balance::ActiveModel {
            id: Set(Uuid::new_v4()),
            purchase_identity: Set(identity.to_string()),
            subscription_credits: Set(monthly_allocation),
            subscription_monthly_allocation: Set(monthly_allocation),
            subscription_resets_at: Set(Some(next_month)),
            extra_credits_remaining: Set(extra_credits), // Sync from purchases!
            last_updated: Set(now),
            created_at: Set(now),
        };

        entity::user_credit_balance::Entity::insert(new_balance)
            .on_conflict(
                OnConflict::column(entity::user_credit_balance::Column::PurchaseIdentity)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(txn)
            .await?;

        entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::PurchaseIdentity.eq(identity))
            .lock_exclusive()
            .one(txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to create or lock credit balance record"
                ))
            })
    }

    /// Helper: Reset subscription credits if needed (outside transaction)
    async fn reset_subscription_if_needed(
        &self,
        balance: entity::user_credit_balance::Model,
        tier: PurchaseTier,
    ) -> Result<entity::user_credit_balance::Model> {
        let now = time::OffsetDateTime::now_utc();

        // Check if reset is needed
        if let Some(resets_at) = balance.subscription_resets_at {
            if now >= resets_at {
                // Reset subscription credits
                let monthly_allocation = self.get_monthly_allocation(tier);
                let next_reset = now + time::Duration::days(30);

                let mut balance_active: entity::user_credit_balance::ActiveModel =
                    balance.into();
                balance_active.subscription_credits = Set(monthly_allocation);
                balance_active.subscription_resets_at = Set(Some(next_reset));
                balance_active.last_updated = Set(now);

                let updated = balance_active.update(&self.db).await?;

                info!(
                    "Reset subscription credits for identity: {} to {}",
                    updated.purchase_identity, monthly_allocation
                );

                return Ok(updated);
            }
        }

        Ok(balance)
    }

    /// Helper: Reset subscription credits if needed (within transaction)
    async fn reset_subscription_if_needed_tx(
        &self,
        balance: entity::user_credit_balance::Model,
        tier: PurchaseTier,
        txn: &DatabaseTransaction,
    ) -> Result<entity::user_credit_balance::Model> {
        let now = time::OffsetDateTime::now_utc();

        // Check if reset is needed
        if let Some(resets_at) = balance.subscription_resets_at {
            if now >= resets_at {
                // Reset subscription credits
                let monthly_allocation = self.get_monthly_allocation(tier);
                let next_reset = now + time::Duration::days(30);

                let mut balance_active: entity::user_credit_balance::ActiveModel =
                    balance.into();
                balance_active.subscription_credits = Set(monthly_allocation);
                balance_active.subscription_resets_at = Set(Some(next_reset));
                balance_active.last_updated = Set(now);

                let updated = balance_active.update(txn).await?;

                info!(
                    "Reset subscription credits for identity: {} to {}",
                    updated.purchase_identity, monthly_allocation
                );

                return Ok(updated);
            }
        }

        Ok(balance)
    }

    /// Helper: Get or create usage record for a date (daily analytics log).
    async fn get_or_create_usage(
        &self,
        identity: &str,
        date: time::Date,
    ) -> Result<entity::quota_usage::Model> {
        let now = time::OffsetDateTime::now_utc();

        // Try to insert a row; if it already exists, do nothing.
        let new_usage = entity::quota_usage::ActiveModel {
            id: Set(Uuid::new_v4()),
            purchase_identity: Set(identity.to_string()),
            usage_date: Set(date),
            text_count: Set(0),
            image_count: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            extra_credits_total: Set(0), // Not used with new architecture
            subscription_credits: Set(0), // Not used with new architecture
            subscription_monthly_allocation: Set(0), // Not used with new architecture
            last_extra_credits_sync: NotSet,
            subscription_resets_at: NotSet,
        };

        // Using ON CONFLICT DO NOTHING avoids unique violations under concurrency.
        entity::quota_usage::Entity::insert(new_usage)
            .on_conflict(
                OnConflict::columns([
                    entity::quota_usage::Column::PurchaseIdentity,
                    entity::quota_usage::Column::UsageDate,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&self.db)
            .await?;

        // Return the existing or newly-inserted row.
        entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(identity))
            .filter(entity::quota_usage::Column::UsageDate.eq(date))
            .one(&self.db)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to find quota usage record after upsert"
                ))
            })
    }

    /// Helper: Find and lock usage record for update
    async fn find_and_lock_usage(
        &self,
        identity: &str,
        date: time::Date,
        txn: &DatabaseTransaction,
    ) -> Result<entity::quota_usage::Model> {
        // Try to find with lock
        let usage = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(identity))
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
            purchase_identity: Set(identity.to_string()),
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
                    entity::quota_usage::Column::PurchaseIdentity,
                    entity::quota_usage::Column::UsageDate,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(txn)
            .await?;

        entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(identity))
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
