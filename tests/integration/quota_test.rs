use backvonia::{config::QuotaConfig, models::common::AIOperation, services::QuotaService};
use entity::sea_orm_active_enums::AccountTier;
use entity::user_credit_balance;
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter,
};
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
        free_text_daily_limit: 15,
        pro_text_daily_limit: 5000,
    }
}

async fn seed_user_balance(
    db: &DatabaseConnection,
    user_id: Uuid,
    tier: &AccountTier,
    config: &QuotaConfig,
) {
    let now = time::OffsetDateTime::now_utc();
    let allocation = match tier {
        AccountTier::Free => config.free_text_daily_limit,
        AccountTier::Pro => config.pro_text_daily_limit,
    };

    let balance = user_credit_balance::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        subscription_credits: Set(allocation),
        subscription_monthly_allocation: Set(allocation),
        subscription_resets_at: Set(Some(now + time::Duration::days(30))),
        extra_credits_remaining: Set(0),
        last_updated: Set(now),
        created_at: Set(now),
    };

    user_credit_balance::Entity::insert(balance)
        .exec(db)
        .await
        .expect("Failed to seed user_credit_balance");
}

async fn get_balance(db: &DatabaseConnection, user_id: Uuid) -> (i32, i32, i32) {
    let balance = user_credit_balance::Entity::find()
        .filter(user_credit_balance::Column::UserId.eq(user_id))
        .one(db)
        .await
        .expect("Failed to query user_credit_balance")
        .expect("Expected user_credit_balance to exist");

    let total = balance.subscription_credits + balance.extra_credits_remaining;
    (
        balance.subscription_credits,
        balance.extra_credits_remaining,
        total,
    )
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_quota_race_condition_prevented() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = Arc::new(QuotaService::new(db.clone(), &config));

    // Test identity with free tier (15 credits from subscription)
    // Each ContinueProse operation costs 5 credits, so should allow 3 operations
    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    seed_user_balance(&db, user_id, &tier, &config).await;

    // Spawn 10 concurrent requests
    let barrier = Arc::new(Barrier::new(10));
    let mut handles = vec![];

    for _ in 0..10 {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        let tier_clone = tier.clone();

        let handle: tokio::task::JoinHandle<backvonia::error::Result<()>> =
            tokio::spawn(async move {
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
    let results: Vec<backvonia::error::Result<()>> = futures::future::join_all(handles)
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
    let service = QuotaService::new(db.clone(), &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    seed_user_balance(&db, user_id, &tier, &config).await;

    // First operation should succeed (ContinueProse = 5 credits)
    let result1 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result1.is_ok());
    let (_, _, total1) = get_balance(&db, user_id).await;
    assert_eq!(total1, 10); // 15 - 5 = 10

    // Second operation should succeed
    let result2 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result2.is_ok());
    let (_, _, total2) = get_balance(&db, user_id).await;
    assert_eq!(total2, 5); // 10 - 5 = 5

    // Third operation should succeed
    let result3 = service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await;
    assert!(result3.is_ok());
    let (_, _, total3) = get_balance(&db, user_id).await;
    assert_eq!(total3, 0); // 5 - 5 = 0

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

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Pro;

    seed_user_balance(&db, user_id, &tier, &config).await;

    // Pro tier should have higher subscription credits (configured)
    let (sub, extra, total) = get_balance(&db, user_id).await;
    assert_eq!(sub, 5000);
    assert_eq!(extra, 0);
    assert_eq!(total, 5000);

    println!("✅ Pro tier quota limits test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_refund_quota_after_failure() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db.clone(), &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    seed_user_balance(&db, user_id, &tier, &config).await;
    let (_, _, initial_total) = get_balance(&db, user_id).await;
    assert_eq!(initial_total, 15); // Free tier starts with 15 credits

    // Deduct credits for image generation (10 credits)
    service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    let (_, _, after_deduct_total) = get_balance(&db, user_id).await;
    assert_eq!(after_deduct_total, 5); // 15 - 10 = 5

    // Simulate failure and refund
    service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Verify refund
    let (_, _, after_refund_total) = get_balance(&db, user_id).await;
    assert_eq!(after_refund_total, 15); // Back to 15

    println!("✅ Refund quota after failure test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_refund_does_not_create_negative_usage() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db.clone(), &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    seed_user_balance(&db, user_id, &tier, &config).await;

    // Try to refund without any prior deduction
    // This should succeed (defensive programming - just adds credits)
    let result = service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await;

    assert!(
        result.is_ok(),
        "Refund should not fail even without prior deduction"
    );

    // Check that credits were added
    let (_, _, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 25); // 15 initial + 10 refunded

    println!("✅ Refund without negative usage test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_refund_multiple_operations() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db.clone(), &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    seed_user_balance(&db, user_id, &tier, &config).await;

    // Deduct for text operation (5 credits)
    service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await
        .unwrap();

    // Deduct for image operation (10 credits)
    service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Should have 0 credits left (15 - 5 - 10 = 0)
    let (_, _, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 0);

    // Refund text operation
    service
        .refund_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await
        .unwrap();

    // Should have 5 credits back
    let (_, _, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 5);

    // Refund image operation
    service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Should have all 15 credits back
    let (_, _, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 15);

    println!("✅ Refund multiple operations test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_refund_with_extra_credits() {
    let db = setup_test_db().await;
    let config = create_test_quota_config();
    let service = QuotaService::new(db.clone(), &config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // Seed extra credits (like from a purchase)
    let now = time::OffsetDateTime::now_utc();
    let balance = user_credit_balance::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        subscription_credits: Set(config.free_text_daily_limit),
        subscription_monthly_allocation: Set(config.free_text_daily_limit),
        subscription_resets_at: Set(Some(now + time::Duration::days(30))),
        extra_credits_remaining: Set(50),
        last_updated: Set(now),
        created_at: Set(now),
    };
    user_credit_balance::Entity::insert(balance)
        .exec(&db)
        .await
        .expect("Failed to seed user_credit_balance");

    // Should have 65 total credits (15 subscription + 50 extra)
    let (sub, extra, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 65);
    assert_eq!(sub, 15);
    assert_eq!(extra, 50);

    // Deduct for image (10 credits from subscription)
    service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Should have 55 total (5 subscription + 50 extra)
    let (sub, extra, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 55);
    assert_eq!(sub, 5);
    assert_eq!(extra, 50);

    // Refund the image operation
    service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Refund goes to extra credits (65 total: 5 subscription + 60 extra)
    let (sub, extra, total) = get_balance(&db, user_id).await;
    assert_eq!(total, 65);
    assert_eq!(sub, 5);
    assert_eq!(extra, 60);

    println!("✅ Refund with extra credits test passed");
}
