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
    pub application: ApplicationConfig,
    pub quota: QuotaConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
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
    pub openai_api_key: String,
    pub anthropic_api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IAPConfig {
    pub apple_shared_secret: String,
    #[serde(default = "default_apple_environment")]
    pub apple_environment: String,
    pub google_service_account_key_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    #[serde(default = "default_access_token_expiration_minutes")]
    pub access_token_expiration_minutes: u64,
    #[serde(default = "default_refresh_token_expiration_days")]
    pub refresh_token_expiration_days: u64,
    pub apple_client_id: String,  // Apple Sign In client ID (bundle ID)
    pub apple_team_id: String,     // Apple developer team ID
    #[serde(default = "default_welcome_bonus_amount")]
    pub welcome_bonus_amount: i32, // Welcome bonus credits for new users (default: 5)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplicationConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaConfig {
    // Limits are expressed in weighted quota units (see AIOperation::cost)
    #[serde(default = "default_free_text_limit")]
    pub free_text_daily_limit: i32,
    #[serde(default = "default_free_image_limit")]
    pub free_image_daily_limit: i32,
    #[serde(default = "default_pro_text_limit")]
    pub pro_text_daily_limit: i32,
    #[serde(default = "default_pro_image_limit")]
    pub pro_image_daily_limit: i32,
}

// Default values
fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_apple_environment() -> String {
    "sandbox".to_string()
}

fn default_access_token_expiration_minutes() -> u64 {
    15 // 15 minutes (short-lived)
}

fn default_refresh_token_expiration_days() -> u64 {
    7 // 7 days
}

fn default_welcome_bonus_amount() -> i32 {
    5 // 5 credits for new users
}

fn default_free_text_limit() -> i32 {
    15 // 3 highest-cost text operations (5 units each)
}

fn default_free_image_limit() -> i32 {
    10 // 1 image generation (10 units)
}

fn default_pro_text_limit() -> i32 {
    5000 // 1000 highest-cost text operations
}

fn default_pro_image_limit() -> i32 {
    500 // 50 image generations
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        // Load .env file if it exists
        dotenvy::dotenv().ok();

        let config = config::Config::builder()
            // Server
            .set_default("server.host", default_host())?
            .set_default("server.port", default_port())?
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
            .set_override_option("ai.openai_api_key", env::var("OPENAI_API_KEY").ok())?
            .set_override_option("ai.anthropic_api_key", env::var("ANTHROPIC_API_KEY").ok())?
            // IAP
            .set_override_option(
                "iap.apple_shared_secret",
                env::var("APPLE_SHARED_SECRET").ok(),
            )?
            .set_override_option("iap.apple_environment", env::var("APPLE_ENVIRONMENT").ok())?
            .set_override_option(
                "iap.google_service_account_key_path",
                env::var("GOOGLE_SERVICE_ACCOUNT_KEY_PATH").ok(),
            )?
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
            .set_override_option("auth.apple_team_id", env::var("APPLE_TEAM_ID").ok())?
            .set_override_option(
                "auth.welcome_bonus_amount",
                env::var("WELCOME_BONUS_AMOUNT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            // Application
            .set_override_option("application.base_url", env::var("BASE_URL").ok())?
            // Quota
            .set_override_option(
                "quota.free_text_daily_limit",
                env::var("FREE_TEXT_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .set_override_option(
                "quota.free_image_daily_limit",
                env::var("FREE_IMAGE_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .set_override_option(
                "quota.pro_text_daily_limit",
                env::var("PRO_TEXT_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .set_override_option(
                "quota.pro_image_daily_limit",
                env::var("PRO_IMAGE_DAILY_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok()),
            )?
            .build()?;

        config.try_deserialize()
    }
}
