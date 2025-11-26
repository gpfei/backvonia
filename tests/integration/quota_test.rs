use backvonia::{
    config::{Config, QuotaConfig},
    models::common::PurchaseTier,
    services::QuotaService,
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use std::sync::Arc;
use tokio::sync::Barrier;

async fn setup_test_db() -> DatabaseConnection {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:dev@localhost:5432/talevonia_test".to_string());

    let db = Database::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations to ensure tables exist
    Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    db
}

fn create_test_quota_config() -> QuotaConfig {
    QuotaConfig {
        free_text_daily_limit: 3,
        free_image_daily_limit: 1,
        pro_text_daily_limit: 1000,
        pro_image_daily_limit: 50,
    }
}

#[tokio::test]
async fn test_quota_race_condition_prevented() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = Arc::new(QuotaService::new(db, &config));

    // Test identity with free tier (limit = 3)
    let identity = format!("test_identity_{}", uuid::Uuid::new_v4());
    let tier = PurchaseTier::Free;

    // Spawn 10 concurrent requests
    let barrier = Arc::new(Barrier::new(10));
    let mut handles = vec![];

    for _ in 0..10 {
        let service = Arc::clone(&service);
        let identity = identity.clone();
        let barrier = Arc::clone(&barrier);

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier.wait().await;

            // Try to increment quota atomically
            service
                .check_and_increment_text_quota(&identity, tier)
                .await
        });

        handles.push(handle);
    }

    // Collect results
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Count successes and failures
    let successes = results.iter().filter(|r| r.is_ok()).count();
    let failures = results.iter().filter(|r| r.is_err()).count();

    // With atomic check+increment, exactly 3 should succeed (free tier limit)
    assert_eq!(successes, 3, "Expected exactly 3 successful quota increments");
    assert_eq!(failures, 7, "Expected 7 quota exceeded errors");

    println!("✅ Quota race condition test passed: {}/10 succeeded", successes);
}

#[tokio::test]
async fn test_quota_check_and_increment_atomic() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db, &config);

    let identity = format!("test_atomic_{}", uuid::Uuid::new_v4());
    let tier = PurchaseTier::Free;

    // First increment should succeed
    let result1 = service
        .check_and_increment_text_quota(&identity, tier)
        .await;
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().text_used, 1);

    // Second increment should succeed
    let result2 = service
        .check_and_increment_text_quota(&identity, tier)
        .await;
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap().text_used, 2);

    // Third increment should succeed
    let result3 = service
        .check_and_increment_text_quota(&identity, tier)
        .await;
    assert!(result3.is_ok());
    assert_eq!(result3.unwrap().text_used, 3);

    // Fourth increment should fail (quota exceeded)
    let result4 = service
        .check_and_increment_text_quota(&identity, tier)
        .await;
    assert!(result4.is_err());

    println!("✅ Atomic quota increment test passed");
}

#[tokio::test]
async fn test_quota_pro_tier_limits() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db, &config);

    let identity = format!("test_pro_{}", uuid::Uuid::new_v4());
    let tier = PurchaseTier::Pro;

    // Pro tier should have higher limits
    let quota_info = service.get_quota_info(&identity, tier).await.unwrap();

    assert_eq!(quota_info.text_limit_daily, 1000);
    assert_eq!(quota_info.image_limit_daily, 50);

    println!("✅ Pro tier quota limits test passed");
}
