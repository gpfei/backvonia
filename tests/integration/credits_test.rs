use backvonia::models::common::IAPPlatform;
use backvonia::models::credit_purchases_ext::CreditPurchaseExt;
use backvonia::services::CreditsService;
use sea_orm::{Database, DatabaseConnection};
use uuid::Uuid;

/// Helper to setup test database
async fn setup_test_db() -> DatabaseConnection {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://myuser:mypassword@192.168.123.187:5432/talevonia".to_string()
    });

    Database::connect(&db_url)
        .await
        .expect("Failed to connect to test database")
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_record_purchase() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = format!("test-user-{}", Uuid::new_v4());
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // Record a purchase
    let result = service
        .record_purchase(
            &user_id,
            Some("original-txn-123"),
            &transaction_id,
            "com.talevonia.tale.credits.500",
            IAPPlatform::Apple,
            500,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await;

    assert!(result.is_ok());
    let (purchase_id, total) = result.unwrap();
    assert_eq!(total, 500);

    // Try to record the same transaction again (should fail with Conflict)
    let duplicate_result = service
        .record_purchase(
            &user_id,
            Some("original-txn-123"),
            &transaction_id,
            "com.talevonia.tale.credits.500",
            IAPPlatform::Apple,
            500,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await;

    assert!(duplicate_result.is_err());
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_consumption_order_subscription_first() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = format!("test-user-{}", Uuid::new_v4());

    // Set up user with BOTH subscription credits AND extra credits
    // This tests the consumption order: subscription FIRST, then extra

    // TODO: Initialize subscription credits (requires quota_usage setup)
    // For now, test with extra credits only

    // Record two extra credit purchases
    let txn1 = format!("txn-{}", Uuid::new_v4());
    let txn2 = format!("txn-{}", Uuid::new_v4());

    // Purchase 1: 100 credits (older)
    service
        .record_purchase(
            &user_id,
            None,
            &txn1,
            "com.talevonia.tale.credits.100",
            IAPPlatform::Apple,
            100,
            time::OffsetDateTime::now_utc() - time::Duration::hours(2),
            None,
        )
        .await
        .expect("Failed to record first purchase");

    // Purchase 2: 500 credits (newer)
    service
        .record_purchase(
            &user_id,
            None,
            &txn2,
            "com.talevonia.tale.credits.500",
            IAPPlatform::Apple,
            500,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await
        .expect("Failed to record second purchase");

    // Verify total credits
    let total = service
        .calculate_total_extra_credits(&user_id)
        .await
        .unwrap();
    assert_eq!(total, 600);

    // Consume 150 credits
    // With NO subscription credits, should consume from extra (FIFO by purchase_date)
    let result = service.consume_credits(&user_id, 150).await;
    assert!(result.is_ok());
    let breakdown = result.unwrap();
    assert_eq!(breakdown.total, 150);
    assert_eq!(breakdown.from_subscription, 0); // No subscription credits
    assert_eq!(breakdown.from_extra, 150);

    // Verify remaining credits
    let remaining = service
        .calculate_total_extra_credits(&user_id)
        .await
        .unwrap();
    assert_eq!(remaining, 450); // 600 - 150

    // Get purchases and verify FIFO consumption within extra credits
    let purchases = service.get_user_purchases(&user_id).await.unwrap();
    assert_eq!(purchases.len(), 2);

    // First purchase (older) should be fully consumed
    assert_eq!(purchases[0].consumed, 100);
    assert_eq!(purchases[0].remaining(), 0);

    // Second purchase (newer) should have 50 consumed
    assert_eq!(purchases[1].consumed, 50);
    assert_eq!(purchases[1].remaining(), 450);
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_insufficient_credits() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = format!("test-user-{}", Uuid::new_v4());

    // Record small purchase
    let txn = format!("txn-{}", Uuid::new_v4());
    service
        .record_purchase(
            &user_id,
            None,
            &txn,
            "com.talevonia.tale.credits.100",
            IAPPlatform::Apple,
            100,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await
        .expect("Failed to record purchase");

    // Try to consume more than available
    let result = service.consume_credits(&user_id, 200).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_get_credits_quota() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = format!("test-user-{}", Uuid::new_v4());

    // Record purchase
    let txn = format!("txn-{}", Uuid::new_v4());
    service
        .record_purchase(
            &user_id,
            None,
            &txn,
            "com.talevonia.tale.credits.500",
            IAPPlatform::Apple,
            500,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await
        .expect("Failed to record purchase");

    // Get quota info
    let quota = service.get_credits_quota(&user_id).await.unwrap();

    assert_eq!(quota.extra_credits.total, 500);
    assert_eq!(quota.extra_credits.purchases.len(), 1);
    assert_eq!(quota.extra_credits.purchases[0].amount, 500);
    assert_eq!(quota.extra_credits.purchases[0].remaining, 500);
    assert_eq!(quota.total_credits, 500);
}
