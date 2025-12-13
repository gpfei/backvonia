use crate::{
    config::Config,
    services::{
        AIService, AuthService, CreditsService, IAPService, JWTService, QuotaService,
        RefreshTokenService, WelcomeBonusService,
    },
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
    pub jwt_service: Arc<JWTService>,
    pub auth_service: Arc<AuthService>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, anyhow::Error> {
        // Connect to database
        let db = sea_orm::Database::connect(&config.database.url).await?;

        // Connect to Redis
        let redis = Arc::new(redis::Client::open(config.redis.url.as_str())?);

        // Wrap config in Arc for sharing
        let config_arc = Arc::new(config);

        // Initialize services
        let ai_service = Arc::new(AIService::new(&config_arc.ai));
        let iap_service = Arc::new(IAPService::new(&config_arc.iap));
        let quota_service = Arc::new(QuotaService::new(db.clone(), &config_arc.quota));
        let credits_service = Arc::new(CreditsService::new(db.clone()));

        // Initialize authentication services
        let auth_config_arc = Arc::new(config_arc.auth.clone());
        let jwt_service = Arc::new(JWTService::new(auth_config_arc.clone()));
        let refresh_token_service = Arc::new(RefreshTokenService::new(
            db.clone(),
            auth_config_arc.clone(),
        ));
        let welcome_bonus_service = Arc::new(WelcomeBonusService::new(db.clone()));
        let auth_service = Arc::new(AuthService::new(
            db.clone(),
            jwt_service.clone(),
            refresh_token_service.clone(),
            welcome_bonus_service.clone(),
            auth_config_arc.clone(),
        ));

        Ok(Self {
            db,
            redis,
            ai_service,
            iap_service,
            quota_service,
            credits_service,
            jwt_service,
            auth_service,
        })
    }
}
