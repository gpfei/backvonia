use crate::{
    config::Config,
    services::{AIService, CreditsService, IAPService, QuotaService},
};
use sea_orm::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub redis: Arc<redis::Client>,
    pub ai_service: Arc<AIService>,
    pub iap_service: Arc<IAPService>,
    pub quota_service: Arc<QuotaService>,
    pub credits_service: Arc<CreditsService>,
    pub config: Arc<Config>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, anyhow::Error> {
        // Connect to database
        let db = sea_orm::Database::connect(&config.database.url).await?;

        // Connect to Redis
        let redis = Arc::new(redis::Client::open(config.redis.url.as_str())?);

        // Initialize services
        let ai_service = Arc::new(AIService::new(&config.ai));
        let iap_service = Arc::new(IAPService::new(&config.iap));
        let quota_service = Arc::new(QuotaService::new(db.clone(), &config.quota));
        let credits_service = Arc::new(CreditsService::new(db.clone()));

        Ok(Self {
            db,
            redis,
            ai_service,
            iap_service,
            quota_service,
            credits_service,
            config: Arc::new(config),
        })
    }
}
