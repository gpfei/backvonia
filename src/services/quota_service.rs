use crate::{
    config::QuotaConfig,
    error::{ApiError, Result},
    models::common::{PurchaseTier, Quota, QuotaSubset},
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
    pub text_used: i32,
    pub text_limit: i32,
    pub image_used: i32,
    pub image_limit: i32,
}

impl QuotaService {
    pub fn new(db: DatabaseConnection, config: &QuotaConfig) -> Self {
        Self {
            db,
            config: config.clone(),
        }
    }

    /// Get daily quota limits for a tier
    fn get_limits(&self, tier: PurchaseTier) -> (i32, i32) {
        match tier {
            PurchaseTier::Free => (
                self.config.free_text_daily_limit,
                self.config.free_image_daily_limit,
            ),
            PurchaseTier::Pro => (
                self.config.pro_text_daily_limit,
                self.config.pro_image_daily_limit,
            ),
        }
    }

    /// Check current quota status for a purchase identity
    #[instrument(skip(self))]
    pub async fn check_quota(&self, identity: &str, tier: PurchaseTier) -> Result<QuotaStatus> {
        let today = time::OffsetDateTime::now_utc().date();
        let (text_limit, image_limit) = self.get_limits(tier);

        // Get or create usage record for today
        let usage = self.get_or_create_usage(identity, today).await?;

        Ok(QuotaStatus {
            text_used: usage.text_count,
            text_limit,
            image_used: usage.image_count,
            image_limit,
        })
    }

    /// Check and increment text quota atomically (prevents race conditions)
    #[instrument(skip(self))]
    pub async fn check_and_increment_text_quota(
        &self,
        identity: &str,
        tier: PurchaseTier,
    ) -> Result<QuotaStatus> {
        let today = time::OffsetDateTime::now_utc().date();
        let (text_limit, image_limit) = self.get_limits(tier);

        let txn = self.db.begin().await?;

        // Lock the row for update
        let usage = self.find_and_lock_usage(identity, today, &txn).await?;

        // Check quota BEFORE incrementing
        if usage.text_count >= text_limit {
            txn.rollback().await?;
            return Err(ApiError::QuotaExceeded(format!(
                "Daily text quota exceeded: {}/{}",
                usage.text_count, text_limit
            )));
        }

        // Increment
        let mut usage_active: entity::quota_usage::ActiveModel = usage.into();
        let current = usage_active.text_count.as_ref().to_owned();
        usage_active.text_count = Set(current + 1);
        usage_active.updated_at = Set(time::OffsetDateTime::now_utc());
        let updated = usage_active.update(&txn).await?;

        txn.commit().await?;

        info!(
            "Atomically checked and incremented text quota for identity: {} ({}/{})",
            identity, updated.text_count, text_limit
        );

        Ok(QuotaStatus {
            text_used: updated.text_count,
            text_limit,
            image_used: updated.image_count,
            image_limit,
        })
    }

    /// Check and increment image quota atomically (prevents race conditions)
    #[instrument(skip(self))]
    pub async fn check_and_increment_image_quota(
        &self,
        identity: &str,
        tier: PurchaseTier,
    ) -> Result<QuotaStatus> {
        let today = time::OffsetDateTime::now_utc().date();
        let (text_limit, image_limit) = self.get_limits(tier);

        let txn = self.db.begin().await?;

        // Lock the row for update
        let usage = self.find_and_lock_usage(identity, today, &txn).await?;

        // Check quota BEFORE incrementing
        if usage.image_count >= image_limit {
            txn.rollback().await?;
            return Err(ApiError::QuotaExceeded(format!(
                "Daily image quota exceeded: {}/{}",
                usage.image_count, image_limit
            )));
        }

        // Increment
        let mut usage_active: entity::quota_usage::ActiveModel = usage.into();
        let current = usage_active.image_count.as_ref().to_owned();
        usage_active.image_count = Set(current + 1);
        usage_active.updated_at = Set(time::OffsetDateTime::now_utc());
        let updated = usage_active.update(&txn).await?;

        txn.commit().await?;

        info!(
            "Atomically checked and incremented image quota for identity: {} ({}/{})",
            identity, updated.image_count, image_limit
        );

        Ok(QuotaStatus {
            text_used: updated.text_count,
            text_limit,
            image_used: updated.image_count,
            image_limit,
        })
    }

    /// Check if text quota is available
    pub async fn can_use_text_quota(&self, identity: &str, tier: PurchaseTier) -> Result<bool> {
        let status = self.check_quota(identity, tier).await?;
        Ok(status.text_used < status.text_limit)
    }

    /// Check if image quota is available
    pub async fn can_use_image_quota(&self, identity: &str, tier: PurchaseTier) -> Result<bool> {
        let status = self.check_quota(identity, tier).await?;
        Ok(status.image_used < status.image_limit)
    }

    /// Get full quota info
    pub async fn get_quota_info(&self, identity: &str, tier: PurchaseTier) -> Result<Quota> {
        let status = self.check_quota(identity, tier).await?;

        Ok(Quota {
            text_limit_daily: status.text_limit,
            text_used_today: status.text_used,
            text_remaining_today: status.text_limit - status.text_used,
            image_limit_daily: status.image_limit,
            image_used_today: status.image_used,
            image_remaining_today: status.image_limit - status.image_used,
        })
    }

    /// Get quota subset for responses
    pub async fn get_quota_subset(
        &self,
        identity: &str,
        tier: PurchaseTier,
    ) -> Result<QuotaSubset> {
        let status = self.check_quota(identity, tier).await?;

        Ok(QuotaSubset {
            text_remaining_today: status.text_limit - status.text_used,
            image_remaining_today: status.image_limit - status.image_used,
        })
    }

    /// Helper: Get or create usage record for a date.
    /// Uses an upsert to avoid unique constraint errors on concurrent first requests.
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
