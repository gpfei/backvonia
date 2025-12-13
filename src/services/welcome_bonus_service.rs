use crate::{error::Result, services::credits_service::CreditsService};
use sea_orm::{entity::*, DatabaseConnection, PaginatorTrait, QueryFilter, TransactionTrait};
use tracing::{info, instrument};
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
        let provider_bonus_count = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::EventType.eq("welcome_bonus"))
            .filter(entity::credits_events::Column::Provider.eq(provider))
            .filter(entity::credits_events::Column::ProviderUserId.eq(provider_user_id))
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
        let device_bonus_count = entity::credits_events::Entity::find()
            .filter(entity::credits_events::Column::EventType.eq("welcome_bonus"))
            .filter(entity::credits_events::Column::DeviceId.eq(device_id))
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
    /// This method atomically records the welcome bonus event in the ledger and applies credits.
    #[instrument(skip(self))]
    pub async fn grant_bonus(
        &self,
        user_id: Uuid,
        device_id: &str,
        provider: &str,
        provider_user_id: &str,
        amount: i32,
    ) -> Result<()> {
        // Start transaction - both bonus record AND credits will be in same txn
        let txn = self.db.begin().await?;

        let now = time::OffsetDateTime::now_utc();
        let (event_id, granted_amount) = self
            .credits_service
            .record_welcome_bonus_in_txn(
                user_id,
                device_id,
                provider,
                provider_user_id,
                amount,
                now,
                &txn,
            )
            .await?;

        // Commit both the bonus record AND the credits atomically
        txn.commit().await?;

        info!(
            user_id = %user_id,
            event_id = %event_id,
            amount = granted_amount,
            "Welcome bonus granted successfully (bonus record + credits committed atomically)"
        );

        Ok(())
    }
}
