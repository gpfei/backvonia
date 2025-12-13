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
            platform: Set(Some(
                match platform {
                    IAPPlatform::Apple => "apple",
                    IAPPlatform::Google => "google",
                }
                .to_string(),
            )),
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

        // Insert purchase idempotently
        entity::credits_events::Entity::insert(new_purchase)
            .on_conflict(
                OnConflict::column(entity::credits_events::Column::TransactionId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;

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

        // Recalculate totals (idempotent: same result if already existed)
        let total_extra = self.recalculate_extra_credits_txn(user_id, &txn).await?;
        txn.commit().await?;

        info!(
            "Recorded credit purchase: user={}, transaction={}, amount={}, total_extra={}",
            user_id, transaction_id, amount, total_extra
        );

        Ok((persisted_purchase.id, total_extra))
    }

    /// Record a welcome bonus event within an existing transaction
    #[instrument(skip(self, txn))]
    #[allow(clippy::too_many_arguments)]
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

        // First check if welcome bonus already exists
        if let Some(existing) = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::TransactionId.eq(&transaction_id))
            .one(txn)
            .await?
        {
            return Err(ApiError::BadRequest(format!(
                "Welcome bonus already granted at {}",
                existing.verified_at
            )));
        }

        // Insert welcome bonus - we've already verified it doesn't exist
        entity::credits_events::Entity::insert(new_event)
            .exec(txn)
            .await?;

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

        // Verify we got the record we just inserted
        assert_eq!(
            persisted.id, event_id,
            "Welcome bonus ID mismatch after insert"
        );

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
