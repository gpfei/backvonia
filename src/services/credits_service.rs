use crate::{
    error::{ApiError, Result},
    models::{
        common::IAPPlatform,
        credit_events_ext::CreditEventExt,
        credits::{
            CreditPurchaseRecord, CreditsQuotaInfo, CreditsQuotaSummary, ExtraCreditsInfo,
            SubscriptionCreditsInfo,
        },
    },
};
use anyhow::anyhow;
use sea_orm::{
    entity::*, query::*, sea_query::OnConflict, DatabaseConnection, DatabaseTransaction,
    TransactionTrait,
};
use tracing::{info, instrument};
use uuid::Uuid;

pub struct CreditsService {
    db: DatabaseConnection,
}

impl CreditsService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record a new credit purchase
    #[instrument(skip(self, receipt_data))]
    #[allow(clippy::too_many_arguments)]
    pub async fn record_purchase(
        &self,
        user_id: Uuid,
        original_transaction_id: Option<&str>,
        transaction_id: &str,
        product_id: &str,
        platform: IAPPlatform,
        amount: i32,
        purchase_date: time::OffsetDateTime,
        receipt_data: Option<&str>,
    ) -> Result<(uuid::Uuid, i32)> {
        // Start transaction
        let txn = self.db.begin().await?;

        // Prepare new purchase record
        let now = time::OffsetDateTime::now_utc();
        let purchase_id = Uuid::new_v4();

        let new_purchase = entity::credits_events::ActiveModel {
            id: Set(purchase_id),
            user_id: Set(user_id),
            event_type: Set("purchase".to_string()),
            original_transaction_id: Set(original_transaction_id.map(|s| s.to_string())),
            transaction_id: Set(transaction_id.to_string()),
            product_id: Set(Some(product_id.to_string())),
            platform: Set(Some(platform.as_str().to_string())),
            amount: Set(amount),
            consumed: Set(0),
            occurred_at: Set(purchase_date),
            verified_at: Set(now),
            receipt_data: Set(receipt_data.map(|s| s.to_string())),
            revoked_at: Set(None),
            revoked_reason: Set(None),
            device_id: Set(None),
            provider: Set(None),
            provider_user_id: Set(None),
            metadata: Set(None),
        };

        // Insert purchase atomically; if the transaction_id already exists, do nothing instead of erroring.
        entity::credits_events::Entity::insert(new_purchase)
            .on_conflict(
                OnConflict::column(entity::credits_events::Column::TransactionId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;

        // Check whether this purchase was inserted or already existed
        let persisted_purchase = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::TransactionId.eq(transaction_id))
            .one(&txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow!(
                    "Failed to read purchase after insert for transaction {}",
                    transaction_id
                ))
            })?;

        if persisted_purchase.id != purchase_id {
            // Another transaction already created this record
            txn.rollback().await?;
            return Err(ApiError::Conflict(format!(
                "Transaction {} already processed at {}",
                transaction_id, persisted_purchase.verified_at
            )));
        }

        // Successfully inserted - recalculate totals
        let total_extra = self.recalculate_extra_credits_txn(user_id, &txn).await?;
        txn.commit().await?;

        info!(
            "Recorded credit purchase: user={}, transaction={}, amount={}, total_extra={}",
            user_id, transaction_id, amount, total_extra
        );

        Ok((purchase_id, total_extra))
    }

    /// Record a new credit purchase within an existing transaction
    /// Used by services that need to atomically combine bonus tracking with credit grants
    #[instrument(skip(self, receipt_data, txn))]
    #[allow(clippy::too_many_arguments)]
    pub async fn record_purchase_in_txn(
        &self,
        user_id: Uuid,
        original_transaction_id: Option<&str>,
        transaction_id: &str,
        product_id: &str,
        platform: IAPPlatform,
        amount: i32,
        purchase_date: time::OffsetDateTime,
        receipt_data: Option<&str>,
        txn: &DatabaseTransaction,
    ) -> Result<(uuid::Uuid, i32)> {
        // Prepare new purchase record
        let now = time::OffsetDateTime::now_utc();
        let purchase_id = Uuid::new_v4();

        let new_purchase = entity::credits_events::ActiveModel {
            id: Set(purchase_id),
            user_id: Set(user_id),
            event_type: Set("purchase".to_string()),
            original_transaction_id: Set(original_transaction_id.map(|s| s.to_string())),
            transaction_id: Set(transaction_id.to_string()),
            product_id: Set(Some(product_id.to_string())),
            platform: Set(Some(platform.as_str().to_string())),
            amount: Set(amount),
            consumed: Set(0),
            occurred_at: Set(purchase_date),
            verified_at: Set(now),
            receipt_data: Set(receipt_data.map(|s| s.to_string())),
            revoked_at: Set(None),
            revoked_reason: Set(None),
            device_id: Set(None),
            provider: Set(None),
            provider_user_id: Set(None),
            metadata: Set(None),
        };

        // Insert purchase atomically; if the transaction_id already exists, do nothing
        entity::credits_events::Entity::insert(new_purchase)
            .on_conflict(
                OnConflict::column(entity::credits_events::Column::TransactionId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(txn)
            .await?;

        // Check whether this purchase was inserted or already existed
        let persisted_purchase = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::TransactionId.eq(transaction_id))
            .one(txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow!(
                    "Failed to read purchase after insert for transaction {}",
                    transaction_id
                ))
            })?;

        if persisted_purchase.id != purchase_id {
            // Another transaction already created this record
            return Err(ApiError::Conflict(format!(
                "Transaction {} already processed at {}",
                transaction_id, persisted_purchase.verified_at
            )));
        }

        // Successfully inserted - recalculate totals
        let total_extra = self.recalculate_extra_credits_txn(user_id, txn).await?;

        info!(
            "Recorded credit purchase in txn: user={}, transaction={}, amount={}, total_extra={}",
            user_id, transaction_id, amount, total_extra
        );

        Ok((purchase_id, total_extra))
    }

    /// Record a welcome bonus event within an existing transaction
    #[instrument(skip(self, txn))]
    pub async fn record_welcome_bonus_in_txn(
        &self,
        user_id: Uuid,
        device_id: &str,
        provider: &str,
        provider_user_id: &str,
        amount: i32,
        granted_at: time::OffsetDateTime,
        txn: &DatabaseTransaction,
    ) -> Result<(uuid::Uuid, i32)> {
        let event_id = Uuid::new_v4();
        let transaction_id = format!("welcome-bonus-{}", user_id);

        let new_event = entity::credits_events::ActiveModel {
            id: Set(event_id),
            user_id: Set(user_id),
            event_type: Set("welcome_bonus".to_string()),
            original_transaction_id: Set(None),
            transaction_id: Set(transaction_id.clone()),
            product_id: Set(Some("com.talevonia.welcome.bonus".to_string())),
            platform: Set(Some(provider.to_string())),
            amount: Set(amount),
            consumed: Set(0),
            occurred_at: Set(granted_at),
            verified_at: Set(granted_at),
            receipt_data: Set(None),
            revoked_at: Set(None),
            revoked_reason: Set(None),
            device_id: Set(Some(device_id.to_string())),
            provider: Set(Some(provider.to_string())),
            provider_user_id: Set(Some(provider_user_id.to_string())),
            metadata: Set(None),
        };

        // Map uniqueness violations to a client-friendly error
        let insert_result = entity::credits_events::Entity::insert(new_event)
            .on_conflict(
                OnConflict::column(entity::credits_events::Column::TransactionId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(txn)
            .await;

        if let Err(ref e) = insert_result {
            let msg = e.to_string();
            if msg.contains("unique") || msg.contains("duplicate") {
                return Err(ApiError::BadRequest(
                    "Welcome bonus already granted".to_string(),
                ));
            }
        }

        insert_result?;

        let persisted = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::TransactionId.eq(transaction_id))
            .one(txn)
            .await?
            .ok_or_else(|| {
                ApiError::Internal(anyhow!(
                    "Failed to read welcome bonus event after insert for user {}",
                    user_id
                ))
            })?;

        if persisted.id != event_id {
            return Err(ApiError::BadRequest(
                "Welcome bonus already granted".to_string(),
            ));
        }

        let total_extra = self.recalculate_extra_credits_txn(user_id, txn).await?;

        Ok((event_id, total_extra))
    }

    /// Get all purchases for a user (ordered by purchase_date for FIFO)
    #[instrument(skip(self))]
    pub async fn get_user_purchases(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<entity::credits_events::Model>> {
        let purchases = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::UserId.eq(user_id))
            .filter(entity::credits_events::Column::EventType.eq("purchase"))
            .filter(entity::credits_events::Column::RevokedAt.is_null())
            .order_by_asc(entity::credits_events::Column::OccurredAt)
            .all(&self.db)
            .await?;

        Ok(purchases)
    }

    /// Calculate total extra credits for a user
    #[instrument(skip(self))]
    pub async fn calculate_total_extra_credits(&self, user_id: Uuid) -> Result<i32> {
        let events = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::UserId.eq(user_id))
            .filter(entity::credits_events::Column::RevokedAt.is_null())
            .filter(entity::credits_events::Column::EventType.ne("consumption"))
            .all(&self.db)
            .await?;

        let total: i32 = events.iter().map(|e| e.remaining()).sum();
        Ok(total)
    }

    /// Recalculate and update extra_credits_remaining in user_credit_balance
    /// This is the CRITICAL method that makes purchased credits usable by QuotaService
    async fn recalculate_extra_credits_txn(
        &self,
        user_id: Uuid,
        txn: &DatabaseTransaction,
    ) -> Result<i32> {
        // Calculate total remaining from additive events (purchases, welcome bonuses, adjustments)
        let events = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::UserId.eq(user_id))
            .filter(entity::credits_events::Column::RevokedAt.is_null())
            .filter(entity::credits_events::Column::EventType.ne("consumption"))
            .all(txn)
            .await?;

        let total_remaining: i32 = events.iter().map(|p| p.remaining()).sum();
        let now = time::OffsetDateTime::now_utc();

        // Upsert user_credit_balance so grants are immediately reflected
        let balance = entity::user_credit_balance::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(user_id),
            subscription_credits: Set(0),
            subscription_monthly_allocation: Set(0),
            subscription_resets_at: Set(None),
            extra_credits_remaining: Set(total_remaining),
            last_updated: Set(now),
            created_at: Set(now),
        };

        entity::user_credit_balance::Entity::insert(balance)
            .on_conflict(
                OnConflict::column(entity::user_credit_balance::Column::UserId)
                    .update_columns([
                        entity::user_credit_balance::Column::ExtraCreditsRemaining,
                        entity::user_credit_balance::Column::LastUpdated,
                    ])
                    .to_owned(),
            )
            .exec(txn)
            .await?;

        Ok(total_remaining)
    }

    /// Consume credits using correct order: subscription FIRST, then extra credits
    ///
    /// Rationale: Subscription credits expire monthly, extra credits never expire.
    /// Therefore, consume subscription credits first to avoid losing them.
    #[instrument(skip(self))]
    pub async fn consume_credits(
        &self,
        user_id: Uuid,
        amount: i32,
    ) -> Result<ConsumedCreditsBreakdown> {
        let txn = self.db.begin().await?;

        let mut remaining = amount;
        let mut consumed_from_subscription = 0;
        let mut consumed_from_extra = 0;

        let today = time::OffsetDateTime::now_utc().date();

        // 1. FIRST: Consume from subscription credits (they expire monthly)
        let quota = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::UserId.eq(user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(today))
            .lock_exclusive()
            .one(&txn)
            .await?;

        if let Some(quota) = quota {
            if quota.subscription_credits > 0 {
                let to_consume = remaining.min(quota.subscription_credits);

                let mut quota_active: entity::quota_usage::ActiveModel = quota.into();
                let current = quota_active.subscription_credits.as_ref().to_owned();
                quota_active.subscription_credits = Set(current - to_consume);
                quota_active.updated_at = Set(time::OffsetDateTime::now_utc());
                quota_active.update(&txn).await?;

                consumed_from_subscription = to_consume;
                remaining -= to_consume;
            }
        }

        // 2. SECOND: If subscription exhausted, consume from extra credits (FIFO by occurred_at)
        if remaining > 0 {
            let events = entity::credits_events::Entity::find()
                .filter(entity::credits_events::Column::UserId.eq(user_id))
                .filter(entity::credits_events::Column::RevokedAt.is_null())
                .filter(entity::credits_events::Column::EventType.ne("consumption"))
                .order_by_asc(entity::credits_events::Column::OccurredAt)
                .lock_exclusive()
                .all(&txn)
                .await?;

            for event in events {
                if remaining == 0 {
                    break;
                }

                let available = event.remaining();
                if available > 0 {
                    let to_consume = remaining.min(available);

                    // Update consumed count
                    let mut event_active: entity::credits_events::ActiveModel = event.into();
                    let current_consumed = event_active.consumed.as_ref().to_owned();
                    event_active.consumed = Set(current_consumed + to_consume);
                    event_active.update(&txn).await?;

                    consumed_from_extra += to_consume;
                    remaining -= to_consume;
                }
            }
        }

        // 3. Check if we have enough credits
        if remaining > 0 {
            txn.rollback().await?;
            return Err(ApiError::QuotaExceeded(format!(
                "Insufficient credits: needed {}, have {} subscription + {} extra = {} total",
                amount,
                consumed_from_subscription,
                consumed_from_extra,
                consumed_from_subscription + consumed_from_extra
            )));
        }

        // 4. Recalculate total extra credits
        self.recalculate_extra_credits_txn(user_id, &txn).await?;

        txn.commit().await?;

        info!(
            "Consumed {} credits for user {}: {} from subscription, {} from extra",
            amount, user_id, consumed_from_subscription, consumed_from_extra
        );

        Ok(ConsumedCreditsBreakdown {
            total: amount,
            from_subscription: consumed_from_subscription,
            from_extra: consumed_from_extra,
        })
    }

    /// Check if user has sufficient credits
    /// CRITICAL FIX: Now reads from user_credit_balance
    #[instrument(skip(self))]
    pub async fn check_sufficient_credits(&self, user_id: Uuid, required: i32) -> Result<bool> {
        // Read from user_credit_balance (accurate balance)
        let balance = entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::UserId.eq(user_id))
            .one(&self.db)
            .await?;

        let total = if let Some(balance) = balance {
            balance.subscription_credits + balance.extra_credits_remaining
        } else {
            0
        };

        Ok(total >= required)
    }

    /// Get complete credits quota information
    /// CRITICAL FIX: Now reads from user_credit_balance instead of quota_usage
    #[instrument(skip(self))]
    pub async fn get_credits_quota(&self, user_id: Uuid) -> Result<CreditsQuotaInfo> {
        // CRITICAL FIX: Read from user_credit_balance (where QuotaService updates balances)
        // NOT from quota_usage (which is only used for daily analytics now)
        let balance = entity::user_credit_balance::Entity::find()
            .filter(entity::user_credit_balance::Column::UserId.eq(user_id))
            .one(&self.db)
            .await?
            .unwrap_or_else(|| {
                // Default balance if user doesn't exist yet
                let now = time::OffsetDateTime::now_utc();
                entity::user_credit_balance::Model {
                    id: Uuid::new_v4(),
                    user_id,
                    subscription_credits: 0,
                    subscription_monthly_allocation: 0,
                    subscription_resets_at: None,
                    extra_credits_remaining: 0,
                    last_updated: now,
                    created_at: now,
                }
            });

        // Get all purchases for detailed breakdown
        let purchases = self.get_user_purchases(user_id).await?;
        let purchase_records: Vec<CreditPurchaseRecord> = purchases
            .iter()
            .map(|p| CreditPurchaseRecord {
                transaction_id: p.transaction_id.clone(),
                product_id: p.product_id.clone().unwrap_or_default(),
                amount: p.amount,
                consumed: p.consumed,
                remaining: p.remaining(),
                purchase_date: p.occurred_at,
            })
            .collect();

        // Use balance.extra_credits_remaining from user_credit_balance (accurate!)
        let extra_credits_total = balance.extra_credits_remaining;

        Ok(CreditsQuotaInfo {
            subscription_credits: SubscriptionCreditsInfo {
                current: balance.subscription_credits,
                monthly_allocation: balance.subscription_monthly_allocation,
                resets_at: balance.subscription_resets_at,
            },
            extra_credits: ExtraCreditsInfo {
                total: extra_credits_total,
                purchases: purchase_records,
            },
            total_credits: balance.subscription_credits + extra_credits_total,
        })
    }

    /// Get quota summary without purchase breakdown
    #[instrument(skip(self))]
    pub async fn get_credits_quota_summary(&self, user_id: Uuid) -> Result<CreditsQuotaSummary> {
        let full = self.get_credits_quota(user_id).await?;
        Ok(CreditsQuotaSummary {
            subscription_credits: full.subscription_credits,
            extra_credits_total: full.extra_credits.total,
            total_credits: full.total_credits,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConsumedCreditsBreakdown {
    pub total: i32,
    pub from_subscription: i32,
    pub from_extra: i32,
}
