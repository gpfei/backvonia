// Integration tests

mod middleware_test;
mod quota_test;

// Test setup helpers
pub async fn setup_test_environment() {
    // Load test environment variables
    dotenvy::from_filename(".env.test").ok();
}
