use backvonia::models::common::IAPPlatform;
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

    let user_id = Uuid::new_v4();
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // Record a purchase
    let result = service
        .record_purchase(
            user_id,
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

    // Try to record the same transaction again (idempotent: should succeed)
    let duplicate_result = service
        .record_purchase(
            user_id,
            Some("original-txn-123"),
            &transaction_id,
            "com.talevonia.tale.credits.500",
            IAPPlatform::Apple,
            500,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await;

    assert!(duplicate_result.is_ok());
    let (purchase_id_2, total_2) = duplicate_result.unwrap();
    assert_eq!(purchase_id_2, purchase_id);
    assert_eq!(total_2, 500);

    let purchases = service.get_user_purchases(user_id).await.unwrap();
    assert_eq!(purchases.len(), 1);
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_get_credits_quota() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = Uuid::new_v4();

    // Record purchase
    let txn = format!("txn-{}", Uuid::new_v4());
    service
        .record_purchase(
            user_id,
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
    let quota = service.get_credits_quota(user_id).await.unwrap();

    assert_eq!(quota.extra_credits.total, 500);
    assert_eq!(quota.extra_credits.purchases.len(), 1);
    assert_eq!(quota.extra_credits.purchases[0].amount, 500);
    assert_eq!(quota.extra_credits.purchases[0].remaining, 500);
    assert_eq!(quota.total_credits, 500);
}
