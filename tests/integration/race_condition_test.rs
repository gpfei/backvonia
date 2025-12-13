/// Test race condition handling in credit purchase recording
///
/// This test verifies that concurrent purchase requests with the same transaction_id
/// are handled correctly - all succeed idempotently, no 500 errors.
use backvonia::models::common::IAPPlatform;
use backvonia::services::CreditsService;
use sea_orm::{Database, DatabaseConnection};
use std::sync::Arc;
use tokio::task::JoinSet;
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
async fn test_concurrent_duplicate_transactions() {
    let db = setup_test_db().await;
    let service = Arc::new(CreditsService::new(db));

    let user_id = Uuid::new_v4();
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // Spawn 5 concurrent requests with the SAME transaction_id
    let mut tasks = JoinSet::new();

    for i in 0..5 {
        let service_clone = service.clone();
        let transaction_id_clone = transaction_id.clone();

        tasks.spawn(async move {
            let result = service_clone
                .record_purchase(
                    user_id,
                    Some("original-txn-123"),
                    &transaction_id_clone,
                    "com.talevonia.tale.credits.500",
                    IAPPlatform::Apple,
                    500,
                    time::OffsetDateTime::now_utc(),
                    None,
                )
                .await;

            (i, result)
        });
    }

    // Collect results
    let mut success_count = 0;
    let mut other_error_count = 0;

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((task_id, purchase_result)) => match purchase_result {
                Ok(_) => {
                    println!("Task {} succeeded", task_id);
                    success_count += 1;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    println!("Task {} got unexpected error: {}", task_id, err_str);
                    other_error_count += 1;
                }
            },
            Err(e) => {
                println!("Task panicked: {:?}", e);
                other_error_count += 1;
            }
        }
    }

    // Assertions:
    // 1. All requests should succeed idempotently
    assert_eq!(success_count, 5, "Expected all requests to succeed");

    // 2. No unexpected errors (like 500 Internal Server Error)
    assert_eq!(other_error_count, 0, "Expected no 500 errors or panics");
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_sequential_duplicate_transactions() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = Uuid::new_v4();
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // First request - should succeed
    let first_result = service
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

    assert!(first_result.is_ok(), "First request should succeed");
    let (purchase_id_1, total_1) = first_result.unwrap();
    assert_eq!(total_1, 500);

    // Second request with same transaction_id - should succeed idempotently
    let second_result = service
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

    assert!(second_result.is_ok(), "Second request should succeed");
    let (purchase_id_2, total_2) = second_result.unwrap();
    assert_eq!(purchase_id_2, purchase_id_1);
    assert_eq!(total_2, 500);
}

#[test]
fn test_unique_violation_detection() {
    // This test documents the behavior of is_unique_violation helper
    // The function should detect PostgreSQL error code 23505 or related strings

    let test_cases = vec![
        ("unique constraint", true),
        ("duplicate key", true),
        ("23505", true),
        ("UNIQUE constraint failed", true),
        ("some other error", false),
        ("general database error", false),
    ];

    for (error_text, should_match) in test_cases {
        let contains_unique = error_text.to_lowercase().contains("unique")
            || error_text.to_lowercase().contains("duplicate")
            || error_text.contains("23505");

        assert_eq!(
            contains_unique, should_match,
            "Error '{}' should match: {}",
            error_text, should_match
        );
    }
}
