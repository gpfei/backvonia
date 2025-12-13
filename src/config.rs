use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub ai: AIConfig,
    pub iap: IAPConfig,
    pub auth: AuthConfig,
    pub quota: QuotaConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AIConfig {
    pub openrouter: OpenRouterConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub api_base: String,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub app_title: Option<String>,
    pub model_tiers: ModelTiers,
    pub image_models: ImageModels,
    pub ai_routing: AIRoutingConfig,
    pub request_timeout_ms: u64,
    pub retry_attempts: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageModels {
    pub free: ImageModelConfig,
    pub pro: ImageModelConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageModelConfig {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelTiers {
    pub premium: ModelTierConfig,
    pub standard: ModelTierConfig,
    pub light: ModelTierConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelTierConfig {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AIRoutingConfig {
    pub fix_grammar: TaskRouting,
    pub shorten: TaskRouting,
    pub rewrite: TaskRouting,
    pub ideas: TaskRouting,
    pub r#continue: TaskRouting,
    pub expand: TaskRouting,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskRouting {
    pub free_default_tier: String,
    pub pro_default_tier: String,
    #[serde(default)]
    pub downgrade_over_chars: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IAPConfig {
    pub apple_shared_secret: String,
    pub apple_environment: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub access_token_expiration_minutes: u64,
    pub refresh_token_expiration_days: u64,
    pub apple_client_id: String,   // Apple Sign In client ID (bundle ID)
    pub welcome_bonus_amount: i32, // Welcome bonus credits for new users
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaConfig {
    // Limits are expressed in weighted quota units (see AIOperation::cost)
    pub free_text_daily_limit: i32,
    pub pro_text_daily_limit: i32,
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        // Load .env file first (this sets environment variables)
        dotenvy::dotenv().ok();

        // Build config - environment variables take precedence over config file
        let config = config::Config::builder()
            // Start with defaults from config.yaml (optional - allows running without config file)
            .add_source(config::File::with_name("config").required(false))
            .set_override_option("server.host", env::var("HOST").ok())?
            .set_override_option(
                "server.port",
                env::var("PORT").ok().and_then(|v| v.parse::<u16>().ok()),
            )?
            // Database
            .set_override_option("database.url", env::var("DATABASE_URL").ok())?
            // Redis
            .set_override_option("redis.url", env::var("REDIS_URL").ok())?
            // AI
            .set_override_option("ai.openrouter.api_key", env::var("OPENROUTER_API_KEY").ok())?
            .set_override_option(
                "ai.openrouter.api_base",
                env::var("OPENROUTER_API_BASE").ok(),
            )?
            // IAP
            .set_override_option(
                "iap.apple_shared_secret",
                env::var("APPLE_SHARED_SECRET").ok(),
            )?
            .set_override_option("iap.apple_environment", env::var("APPLE_ENVIRONMENT").ok())?
            // Auth
            .set_override_option("auth.jwt_secret", env::var("JWT_SECRET").ok())?
            .set_override_option(
                "auth.access_token_expiration_minutes",
                env::var("ACCESS_TOKEN_EXPIRATION_MINUTES")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok()),
            )?
            .set_override_option(
                "auth.refresh_token_expiration_days",
                env::var("REFRESH_TOKEN_EXPIRATION_DAYS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok()),
            )?
            .set_override_option("auth.apple_client_id", env::var("APPLE_CLIENT_ID").ok())?
            .set_override_option(
                "auth.welcome_bonus_amount",
                env::var("WELCOME_BONUS_AMOUNT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            // Quota
            .set_override_option(
                "quota.free_text_daily_limit",
                env::var("FREE_TEXT_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .set_override_option(
                "quota.pro_text_daily_limit",
                env::var("PRO_TEXT_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .build()?;

        config.try_deserialize()
    }
}
