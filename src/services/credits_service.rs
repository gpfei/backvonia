use crate::{
    error::{ApiError, Result},
    models::{
        common::IAPPlatform,
        credit_purchases_ext::CreditPurchaseExt,
        credits::{
            CreditPurchaseRecord, CreditsQuotaInfo, ExtraCreditsInfo, SubscriptionCreditsInfo,
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
    pub async fn record_purchase(
        &self,
        local_user_id: &str,
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

        let new_purchase = entity::credit_purchases::ActiveModel {
            id: Set(purchase_id),
            local_user_id: Set(local_user_id.to_string()),
            original_transaction_id: Set(original_transaction_id.map(|s| s.to_string())),
            transaction_id: Set(transaction_id.to_string()),
            product_id: Set(product_id.to_string()),
            platform: Set(platform.as_str().to_string()),
            amount: Set(amount),
            consumed: Set(0),
            purchase_date: Set(purchase_date),
            verified_at: Set(now),
            receipt_data: Set(receipt_data.map(|s| s.to_string())),
            revoked_at: Set(None),
            revoked_reason: Set(None),
        };

        // Insert purchase atomically; if the transaction_id already exists, do nothing instead of erroring.
        entity::credit_purchases::Entity::insert(new_purchase)
            .on_conflict(
                OnConflict::column(entity::credit_purchases::Column::TransactionId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;

        // Check whether this purchase was inserted or already existed
        let persisted_purchase = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::TransactionId.eq(transaction_id))
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
        let total_extra = self
            .recalculate_extra_credits_txn(local_user_id, &txn)
            .await?;
        txn.commit().await?;

        info!(
            "Recorded credit purchase: user={}, transaction={}, amount={}, total_extra={}",
            local_user_id, transaction_id, amount, total_extra
        );

        Ok((purchase_id, total_extra))
    }

    /// Get all purchases for a user (ordered by purchase_date for FIFO)
    #[instrument(skip(self))]
    pub async fn get_user_purchases(
        &self,
        local_user_id: &str,
    ) -> Result<Vec<entity::credit_purchases::Model>> {
        let purchases = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::LocalUserId.eq(local_user_id))
            .filter(entity::credit_purchases::Column::RevokedAt.is_null())
            .order_by_asc(entity::credit_purchases::Column::PurchaseDate)
            .all(&self.db)
            .await?;

        Ok(purchases)
    }

    /// Calculate total extra credits for a user
    #[instrument(skip(self))]
    pub async fn calculate_total_extra_credits(&self, local_user_id: &str) -> Result<i32> {
        let purchases = self.get_user_purchases(local_user_id).await?;
        let total: i32 = purchases.iter().map(|p| p.remaining()).sum();
        Ok(total)
    }

    /// Recalculate and update extra_credits_total in quota_usage
    async fn recalculate_extra_credits_txn(
        &self,
        local_user_id: &str,
        txn: &DatabaseTransaction,
    ) -> Result<i32> {
        // Calculate total from purchases
        let purchases = entity::credit_purchases::Entity::find()
            .filter(entity::credit_purchases::Column::LocalUserId.eq(local_user_id))
            .filter(entity::credit_purchases::Column::RevokedAt.is_null())
            .all(txn)
            .await?;

        let total: i32 = purchases.iter().map(|p| p.remaining()).sum();

        // Update quota_usage record
        let today = time::OffsetDateTime::now_utc().date();
        let now = time::OffsetDateTime::now_utc();

        // Find or create quota_usage for today
        let quota_usage = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(local_user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(today))
            .one(txn)
            .await?;

        if let Some(quota) = quota_usage {
            // Update existing
            let mut quota_active: entity::quota_usage::ActiveModel = quota.into();
            quota_active.extra_credits_total = Set(total);
            quota_active.last_extra_credits_sync = Set(Some(now));
            quota_active.updated_at = Set(now);
            quota_active.update(txn).await?;
        } else {
            // Create new record
            let new_quota = entity::quota_usage::ActiveModel {
                id: Set(Uuid::new_v4()),
                purchase_identity: Set(local_user_id.to_string()),
                usage_date: Set(today),
                text_count: Set(0),
                image_count: Set(0),
                extra_credits_total: Set(total),
                subscription_credits: Set(0),
                subscription_monthly_allocation: Set(0),
                last_extra_credits_sync: Set(Some(now)),
                subscription_resets_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };

            entity::quota_usage::Entity::insert(new_quota)
                .on_conflict(
                    OnConflict::columns([
                        entity::quota_usage::Column::PurchaseIdentity,
                        entity::quota_usage::Column::UsageDate,
                    ])
                    .update_columns([
                        entity::quota_usage::Column::ExtraCreditsTotal,
                        entity::quota_usage::Column::LastExtraCreditsSync,
                        entity::quota_usage::Column::UpdatedAt,
                    ])
                    .to_owned(),
                )
                .exec(txn)
                .await?;
        }

        Ok(total)
    }

    /// Consume credits using correct order: subscription FIRST, then extra credits
    ///
    /// Rationale: Subscription credits expire monthly, extra credits never expire.
    /// Therefore, consume subscription credits first to avoid losing them.
    #[instrument(skip(self))]
    pub async fn consume_credits(
        &self,
        local_user_id: &str,
        amount: i32,
    ) -> Result<ConsumedCreditsBreakdown> {
        let txn = self.db.begin().await?;

        let mut remaining = amount;
        let mut consumed_from_subscription = 0;
        let mut consumed_from_extra = 0;

        let today = time::OffsetDateTime::now_utc().date();

        // 1. FIRST: Consume from subscription credits (they expire monthly)
        let quota = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(local_user_id))
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

        // 2. SECOND: If subscription exhausted, consume from extra credits (FIFO by purchase_date)
        if remaining > 0 {
            let purchases = entity::credit_purchases::Entity::find()
                .filter(entity::credit_purchases::Column::LocalUserId.eq(local_user_id))
                .filter(entity::credit_purchases::Column::RevokedAt.is_null())
                .order_by_asc(entity::credit_purchases::Column::PurchaseDate)
                .lock_exclusive()
                .all(&txn)
                .await?;

            for purchase in purchases {
                if remaining == 0 {
                    break;
                }

                let available = purchase.remaining();
                if available > 0 {
                    let to_consume = remaining.min(available);

                    // Update consumed count
                    let mut purchase_active: entity::credit_purchases::ActiveModel =
                        purchase.into();
                    let current_consumed = purchase_active.consumed.as_ref().to_owned();
                    purchase_active.consumed = Set(current_consumed + to_consume);
                    purchase_active.update(&txn).await?;

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
        self.recalculate_extra_credits_txn(local_user_id, &txn)
            .await?;

        txn.commit().await?;

        info!(
            "Consumed {} credits: {} from subscription, {} from extra",
            amount, consumed_from_subscription, consumed_from_extra
        );

        Ok(ConsumedCreditsBreakdown {
            total: amount,
            from_subscription: consumed_from_subscription,
            from_extra: consumed_from_extra,
        })
    }

    /// Check if user has sufficient credits
    #[instrument(skip(self))]
    pub async fn check_sufficient_credits(
        &self,
        local_user_id: &str,
        required: i32,
    ) -> Result<bool> {
        let extra = self.calculate_total_extra_credits(local_user_id).await?;

        let today = time::OffsetDateTime::now_utc().date();
        let quota = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(local_user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(today))
            .one(&self.db)
            .await?;

        let subscription = quota.map(|q| q.subscription_credits).unwrap_or(0);
        let total = extra + subscription;

        Ok(total >= required)
    }

    /// Get complete credits quota information
    #[instrument(skip(self))]
    pub async fn get_credits_quota(&self, local_user_id: &str) -> Result<CreditsQuotaInfo> {
        let today = time::OffsetDateTime::now_utc().date();

        // Get quota_usage record
        let quota = entity::quota_usage::Entity::find()
            .filter(entity::quota_usage::Column::PurchaseIdentity.eq(local_user_id))
            .filter(entity::quota_usage::Column::UsageDate.eq(today))
            .one(&self.db)
            .await?
            .unwrap_or_else(|| entity::quota_usage::Model {
                id: Uuid::new_v4(),
                purchase_identity: local_user_id.to_string(),
                usage_date: today,
                text_count: 0,
                image_count: 0,
                extra_credits_total: 0,
                subscription_credits: 0,
                subscription_monthly_allocation: 0,
                last_extra_credits_sync: None,
                subscription_resets_at: None,
                created_at: time::OffsetDateTime::now_utc(),
                updated_at: time::OffsetDateTime::now_utc(),
            });

        // Get all purchases
        let purchases = self.get_user_purchases(local_user_id).await?;
        let purchase_records: Vec<CreditPurchaseRecord> = purchases
            .iter()
            .map(|p| CreditPurchaseRecord {
                transaction_id: p.transaction_id.clone(),
                product_id: p.product_id.clone(),
                amount: p.amount,
                consumed: p.consumed,
                remaining: p.remaining(),
                purchase_date: p.purchase_date,
            })
            .collect();

        let extra_credits_total: i32 = purchases.iter().map(|p| p.remaining()).sum();

        Ok(CreditsQuotaInfo {
            subscription_credits: SubscriptionCreditsInfo {
                current: quota.subscription_credits,
                monthly_allocation: quota.subscription_monthly_allocation,
                resets_at: quota.subscription_resets_at,
            },
            extra_credits: ExtraCreditsInfo {
                total: extra_credits_total,
                purchases: purchase_records,
            },
            total_credits: quota.subscription_credits + extra_credits_total,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConsumedCreditsBreakdown {
    pub total: i32,
    pub from_subscription: i32,
    pub from_extra: i32,
}
