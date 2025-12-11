use backvonia::{
    config::QuotaConfig,
    models::{
        ai::{
            AIImageGenerateRequest, ImageParams, ImageStoryContext, ImageStyle, NodeContext,
        },
        common::AIOperation,
    },
    services::QuotaService,
};
use entity::sea_orm_active_enums::AccountTier;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection, EntityTrait, QueryFilter, ColumnTrait};
use uuid::Uuid;

async fn setup_test_db() -> DatabaseConnection {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:dev@localhost:5432/talevonia_test".to_string()
    });

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

fn create_test_image_request() -> AIImageGenerateRequest {
    AIImageGenerateRequest {
        story_context: ImageStoryContext {
            title: "Test Story".to_string(),
            language: "en".to_string(),
            genre: Some("Fantasy".to_string()),
            tone: Some("Dark".to_string()),
            setting: Some("A mysterious forest".to_string()),
        },
        node: NodeContext {
            summary: Some("The hero discovers a mysterious artifact".to_string()),
            content: Some("Alice stepped into the crumbling hall...".to_string()),
            tags: vec!["discovery".to_string(), "artifact".to_string()],
        },
        image_params: ImageParams {
            style: Some(ImageStyle::Storybook),
            aspect_ratio: "3:4".to_string(),
            resolution: "medium".to_string(),
        },
    }
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_image_generation_failure_refunds_credits() {
    let db = setup_test_db().await;
    let quota_config = create_test_quota_config();
    let quota_service = QuotaService::new(db.clone(), &quota_config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // Check initial credits
    let initial_status = quota_service.check_quota(user_id, &tier).await.unwrap();
    assert_eq!(initial_status.total_credits_remaining, 15); // Free tier: 15 credits

    // Simulate the flow that happens in the route handler:
    // 1. Deduct credits
    let after_deduct = quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();
    assert_eq!(after_deduct.total_credits_remaining, 5); // 15 - 10 = 5

    // 2. Simulate generation failure (in real code, this would be DALL-E failure, upload failure, etc.)
    // Just simulate by calling refund directly

    // 3. Refund credits
    quota_service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // 4. Verify credits were refunded
    let after_refund = quota_service.check_quota(user_id, &tier).await.unwrap();
    assert_eq!(after_refund.total_credits_remaining, 15); // Back to original

    // 5. Verify user can still use the service (credits are available)
    let second_attempt = quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await;
    assert!(second_attempt.is_ok(), "User should be able to retry after refund");

    println!("✅ Image generation failure refund test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_multiple_failures_multiple_refunds() {
    let db = setup_test_db().await;
    let quota_config = create_test_quota_config();
    let quota_service = QuotaService::new(db.clone(), &quota_config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // Simulate 3 failed attempts with refunds
    for i in 1..=3 {
        println!("Attempt {}", i);

        // Check credits before
        let before = quota_service.check_quota(user_id, &tier).await.unwrap();
        assert_eq!(before.total_credits_remaining, 15);

        // Deduct
        quota_service
            .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
            .await
            .unwrap();

        // Verify deduction
        let after_deduct = quota_service.check_quota(user_id, &tier).await.unwrap();
        assert_eq!(after_deduct.total_credits_remaining, 5);

        // Refund
        quota_service
            .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
            .await
            .unwrap();

        // Verify refund
        let after_refund = quota_service.check_quota(user_id, &tier).await.unwrap();
        assert_eq!(after_refund.total_credits_remaining, 15);
    }

    println!("✅ Multiple failures with refunds test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_partial_failure_preserves_successful_operations() {
    let db = setup_test_db().await;
    let quota_config = create_test_quota_config();
    let quota_service = QuotaService::new(db.clone(), &quota_config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // First operation: success (don't refund)
    quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ContinueProse)
        .await
        .unwrap();

    // After first success: 15 - 5 = 10 credits
    let status = quota_service.check_quota(user_id, &tier).await.unwrap();
    assert_eq!(status.total_credits_remaining, 10);

    // Second operation: failure (deduct then refund)
    quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // After second deduct: 10 - 10 = 0
    let status = quota_service.check_quota(user_id, &tier).await.unwrap();
    assert_eq!(status.total_credits_remaining, 0);

    // Refund the failed image operation
    quota_service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // After refund: should have 10 credits (first operation still consumed)
    let status = quota_service.check_quota(user_id, &tier).await.unwrap();
    assert_eq!(status.total_credits_remaining, 10);

    println!("✅ Partial failure preserves successful operations test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_free_tier_single_failure_can_retry() {
    let db = setup_test_db().await;
    let quota_config = create_test_quota_config();
    let quota_service = QuotaService::new(db.clone(), &quota_config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;

    // Free tier has 15 credits, image costs 10
    // After one failure with refund, user should be able to retry

    // First attempt: fail
    quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    quota_service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Second attempt: should succeed (credits available)
    let result = quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await;

    assert!(
        result.is_ok(),
        "Free tier user should be able to retry after refund"
    );

    let status = result.unwrap();
    assert_eq!(status.total_credits_remaining, 5); // 15 - 10 = 5

    println!("✅ Free tier retry after failure test passed");
}

#[tokio::test]
#[ignore] // Run only when test database is available
async fn test_analytics_accuracy_after_refund() {
    let db = setup_test_db().await;
    let quota_config = create_test_quota_config();
    let quota_service = QuotaService::new(db.clone(), &quota_config);

    let user_id = Uuid::new_v4();
    let tier = AccountTier::Free;
    let today = time::OffsetDateTime::now_utc().date();

    // Successful image generation
    quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Check usage record
    let usage = entity::quota_usage::Entity::find()
        .filter(entity::quota_usage::Column::UserId.eq(user_id))
        .filter(entity::quota_usage::Column::UsageDate.eq(today))
        .one(&db)
        .await
        .unwrap()
        .expect("Usage record should exist");

    assert_eq!(usage.image_count, 10); // 10 credits used

    // Failed image generation (deduct then refund)
    quota_service
        .check_and_increment_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    quota_service
        .refund_quota_weighted(user_id, &tier, AIOperation::ImageGenerate)
        .await
        .unwrap();

    // Check usage record - should still be 10 (failed operation refunded)
    let usage = entity::quota_usage::Entity::find()
        .filter(entity::quota_usage::Column::UserId.eq(user_id))
        .filter(entity::quota_usage::Column::UsageDate.eq(today))
        .one(&db)
        .await
        .unwrap()
        .expect("Usage record should exist");

    assert_eq!(
        usage.image_count, 10,
        "Analytics should show net usage (success - refunded)"
    );

    println!("✅ Analytics accuracy after refund test passed");
}
