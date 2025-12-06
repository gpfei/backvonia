use backvonia::{
    config::{Config, QuotaConfig},
    models::common::AIOperation,
    services::QuotaService,
};
use entity::sea_orm_active_enums::AccountTier;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use std::sync::Arc;
use tokio::sync::Barrier;
use uuid::Uuid;

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
#[ignore] // Run only when test database is available
async fn test_quota_race_condition_prevented() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = Arc::new(QuotaService::new(db, &config));

    // Test identity with free tier (15 credits from subscription)
    // Each ContinueProse operation costs 5 credits, so should allow 3 operations
    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // Spawn 10 concurrent requests
    let barrier = Arc::new(Barrier::new(10));
    let mut handles = vec![];

    for _ in 0..10 {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        let tier_clone = tier.clone();

        let handle: tokio::task::JoinHandle<
            backvonia::error::Result<backvonia::services::quota_service::QuotaStatus>,
        > = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier.wait().await;

            // Try to use credits atomically (ContinueProse = 5 credits)
            service
                .check_and_increment_quota_weighted(
                    user_id,
                    &tier_clone,
                    AIOperation::ContinueProse,
                )
                .await
        });

        handles.push(handle);
    }

    // Collect results
    let results: Vec<backvonia::error::Result<backvonia::services::quota_service::QuotaStatus>> =
        futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

    // Count successes and failures
    let successes = results.iter().filter(|r| r.is_ok()).count();
    let failures = results.iter().filter(|r| r.is_err()).count();

    // With atomic check+increment, exactly 3 should succeed (15 credits / 5 per operation)
    assert_eq!(
        successes, 3,
        "Expected exactly 3 successful quota increments"
    );
    assert_eq!(failures, 7, "Expected 7 quota exceeded errors");

    println!(
        "✅ Quota race condition test passed: {}/10 succeeded",
        successes
    );
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_quota_check_and_increment_atomic() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db, &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // First operation should succeed (ContinueProse = 5 credits)
    let result1 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result1.is_ok());
    let status1 = result1.unwrap();
    assert_eq!(status1.total_credits_remaining, 10); // 15 - 5 = 10

    // Second operation should succeed
    let result2 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result2.is_ok());
    let status2 = result2.unwrap();
    assert_eq!(status2.total_credits_remaining, 5); // 10 - 5 = 5

    // Third operation should succeed
    let result3 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result3.is_ok());
    let status3 = result3.unwrap();
    assert_eq!(status3.total_credits_remaining, 0); // 5 - 5 = 0

    // Fourth operation should fail (quota exceeded)
    let result4 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result4.is_err());

    println!("✅ Atomic quota increment test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_quota_pro_tier_limits() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db, &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Pro;

    // Pro tier should have higher subscription credits (5000)
    let quota_info = service.get_quota_info(user_id, &tier).await.unwrap();

    assert_eq!(quota_info.subscription_credits, 5000);
    assert_eq!(quota_info.total_credits_remaining, 5000);

    println!("✅ Pro tier quota limits test passed");
}
