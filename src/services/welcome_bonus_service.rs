use crate::{
    error::{ApiError, Result},
    models::common::IAPPlatform,
    services::credits_service::CreditsService,
};
use sea_orm::{entity::*, DatabaseConnection, PaginatorTrait, QueryFilter, TransactionTrait};
use tracing::{info, instrument, warn};
use uuid::Uuid;

pub struct WelcomeBonusService {
    db: DatabaseConnection,
    credits_service: CreditsService,
}

impl WelcomeBonusService {
    pub fn new(db: DatabaseConnection) -> Self {
        let credits_service = CreditsService::new(db.clone());
        Self {
            db,
            credits_service,
        }
    }

    /// Check if a user is eligible for welcome bonus
    ///
    /// Requirements:
    /// - Device ID must be provided (required)
    /// - Provider account has never received bonus
    /// - No user has received bonus from this device before
    #[instrument(skip(self))]
    pub async fn check_eligibility(
        &self,
        device_id: &str,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<bool> {
        // Device ID is required
        if device_id.is_empty() {
            info!("Welcome bonus denied: device_id not provided");
            return Ok(false);
        }

        // Check if provider account already got bonus
        let provider_bonus_count = entity::welcome_bonuses::Entity::find()
            .filter(entity::welcome_bonuses::Column::Provider.eq(provider))
            .filter(entity::welcome_bonuses::Column::ProviderUserId.eq(provider_user_id))
            .count(&self.db)
            .await?;

        if provider_bonus_count > 0 {
            info!(
                provider = provider,
                provider_user_id = provider_user_id,
                "Welcome bonus denied: provider account already received bonus"
            );
            return Ok(false);
        }

        // Check if device already gave bonus to another user
        let device_bonus_count = entity::welcome_bonuses::Entity::find()
            .filter(entity::welcome_bonuses::Column::DeviceId.eq(device_id))
            .count(&self.db)
            .await?;

        if device_bonus_count > 0 {
            info!(
                device_id = device_id,
                "Welcome bonus denied: device already gave bonus to another user"
            );
            return Ok(false);
        }

        // All checks passed
        info!(
            device_id = device_id,
            provider = provider,
            provider_user_id = provider_user_id,
            "Welcome bonus eligibility check passed"
        );
        Ok(true)
    }

    /// Grant welcome bonus to a user
    ///
    /// This method:
    /// 1. Inserts a record into welcome_bonuses table
    /// 2. Calls credits_service.record_purchase() to add credits
    ///
    /// The method is idempotent - if called multiple times for the same user,
    /// it will fail gracefully due to unique constraints.
    #[instrument(skip(self))]
    pub async fn grant_bonus(
        &self,
        user_id: Uuid,
        device_id: &str,
        provider: &str,
        provider_user_id: &str,
        amount: i32,
    ) -> Result<()> {
        // Start transaction
        let txn = self.db.begin().await?;

        // Insert welcome bonus record
        let bonus_id = Uuid::new_v4();
        let now = time::OffsetDateTime::now_utc();

        let new_bonus = entity::welcome_bonuses::ActiveModel {
            id: Set(bonus_id),
            user_id: Set(user_id),
            device_id: Set(device_id.to_string()),
            provider: Set(provider.to_string()),
            provider_user_id: Set(provider_user_id.to_string()),
            amount_granted: Set(amount),
            reason: Set("new_user".to_string()),
            granted_at: Set(now),
        };

        // Insert into welcome_bonuses
        let result = entity::welcome_bonuses::Entity::insert(new_bonus)
            .exec(&txn)
            .await;

        match result {
            Ok(_) => {
                info!(
                    user_id = %user_id,
                    amount = amount,
                    device_id = device_id,
                    provider = provider,
                    "Welcome bonus record created"
                );
            }
            Err(e) => {
                // Check if this is a unique constraint violation
                // If so, it means the bonus was already granted (race condition or retry)
                let error_msg = e.to_string();
                if error_msg.contains("unique") || error_msg.contains("duplicate") {
                    warn!(
                        user_id = %user_id,
                        device_id = device_id,
                        "Welcome bonus already granted (duplicate attempt)"
                    );
                    return Err(ApiError::BadRequest(
                        "Welcome bonus already granted".to_string(),
                    ));
                }
                return Err(ApiError::from(e));
            }
        }

        // Commit the transaction (bonus record created)
        txn.commit().await?;

        // Record the credit purchase
        // Note: This is done AFTER the bonus record is committed to ensure
        // we don't grant credits without tracking in welcome_bonuses table
        let platform = match provider {
            "apple" => IAPPlatform::Apple,
            "google" => IAPPlatform::Google,
            _ => IAPPlatform::Apple, // Default to Apple
        };

        let transaction_id = format!("welcome-bonus-{}", user_id);
        let product_id = "com.talevonia.welcome.bonus";

        match self
            .credits_service
            .record_purchase(
                user_id,
                None, // No original_transaction_id for welcome bonus
                &transaction_id,
                product_id,
                platform,
                amount,
                now,
                None, // No receipt
            )
            .await
        {
            Ok((purchase_id, granted_amount)) => {
                info!(
                    user_id = %user_id,
                    purchase_id = %purchase_id,
                    amount = granted_amount,
                    "Welcome bonus credits granted successfully"
                );
                Ok(())
            }
            Err(e) => {
                // Credit recording failed after bonus record was created
                // This is a serious issue - log it but don't rollback the bonus record
                // since it's already committed. The user will need manual credit adjustment.
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "Failed to record welcome bonus credits after creating bonus record. Manual credit adjustment needed."
                );
                Err(ApiError::Internal(anyhow::anyhow!(
                    "Welcome bonus recorded but credits grant failed: {}",
                    e
                )))
            }
        }
    }
}
