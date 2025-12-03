// Library exports for testing and reuse
pub mod app_state;
pub mod config;
pub mod error;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;
pub mod utils;

// Re-export commonly used types
pub use app_state::AppState;
pub use config::Config;
pub use error::{ApiError, Result};
