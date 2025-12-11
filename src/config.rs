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
    #[serde(default)]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
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
    pub size: String,
    pub quality: String,
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
    pub max_context_tokens: u32,
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
    #[serde(default)]
    pub google_service_account_key_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub access_token_expiration_minutes: u64,
    pub refresh_token_expiration_days: u64,
    pub apple_client_id: String, // Apple Sign In client ID (bundle ID)
    pub apple_team_id: String,   // Apple developer team ID
    pub welcome_bonus_amount: i32, // Welcome bonus credits for new users
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplicationConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaConfig {
    // Limits are expressed in weighted quota units (see AIOperation::cost)
    pub free_text_daily_limit: i32,
    pub free_image_daily_limit: i32,
    pub pro_text_daily_limit: i32,
    pub pro_image_daily_limit: i32,
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        // Load .env file if it exists (for environment variable overrides)
        dotenvy::dotenv().ok();

        // Build config from config.yml (required) with environment variable overrides
        let config = config::Config::builder()
            // Load config.yml (REQUIRED)
            .add_source(config::File::with_name("config").required(true))
            // Allow environment variables to override config file
            .add_source(
                config::Environment::with_prefix("BACKVONIA")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        config.try_deserialize()
    }
}
