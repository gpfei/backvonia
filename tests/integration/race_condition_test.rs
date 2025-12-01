/// Test race condition handling in credit purchase recording
///
/// This test verifies that concurrent purchase requests with the same transaction_id
/// are handled correctly - one succeeds, others get 409 Conflict, no 500 errors.

use backvonia::models::common::IAPPlatform;
use backvonia::services::CreditsService;
use sea_orm::{Database, DatabaseConnection};
use std::sync::Arc;
use tokio::task::JoinSet;
use uuid::Uuid;

/// Helper to setup test database
async fn setup_test_db() -> DatabaseConnection {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://myuser:mypassword@192.168.123.187:5432/talevonia".to_string());

    Database::connect(&db_url)
        .await
        .expect("Failed to connect to test database")
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_concurrent_duplicate_transactions() {
    let db = setup_test_db().await;
    let service = Arc::new(CreditsService::new(db));

    let user_id = format!("test-user-{}", Uuid::new_v4());
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // Spawn 5 concurrent requests with the SAME transaction_id
    let mut tasks = JoinSet::new();

    for i in 0..5 {
        let service_clone = service.clone();
        let user_id_clone = user_id.clone();
        let transaction_id_clone = transaction_id.clone();

        tasks.spawn(async move {
            let result = service_clone
                .record_purchase(
                    &user_id_clone,
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
    let mut conflict_count = 0;
    let mut other_error_count = 0;

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((task_id, purchase_result)) => {
                match purchase_result {
                    Ok(_) => {
                        println!("Task {} succeeded", task_id);
                        success_count += 1;
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("already processed") || err_str.to_lowercase().contains("conflict") {
                            println!("Task {} got expected Conflict: {}", task_id, err_str);
                            conflict_count += 1;
                        } else {
                            println!("Task {} got unexpected error: {}", task_id, err_str);
                            other_error_count += 1;
                        }
                    }
                }
            }
            Err(e) => {
                println!("Task panicked: {:?}", e);
                other_error_count += 1;
            }
        }
    }

    // Assertions:
    // 1. Exactly ONE request should succeed
    assert_eq!(success_count, 1, "Expected exactly 1 successful insert");

    // 2. All other requests should get Conflict (not 500 errors)
    assert_eq!(conflict_count, 4, "Expected 4 Conflict responses");

    // 3. No unexpected errors (like 500 Internal Server Error)
    assert_eq!(other_error_count, 0, "Expected no 500 errors or panics");
}

#[tokio::test]
#[ignore] // Run only when database is available
async fn test_sequential_duplicate_transactions() {
    let db = setup_test_db().await;
    let service = CreditsService::new(db);

    let user_id = format!("test-user-{}", Uuid::new_v4());
    let transaction_id = format!("txn-{}", Uuid::new_v4());

    // First request - should succeed
    let first_result = service
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

    assert!(first_result.is_ok(), "First request should succeed");
    let (purchase_id_1, total_1) = first_result.unwrap();
    assert_eq!(total_1, 500);

    // Second request with same transaction_id - should get Conflict
    let second_result = service
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

    assert!(second_result.is_err(), "Second request should fail");

    let error = second_result.unwrap_err();
    let error_msg = error.to_string();

    // Should be a Conflict error, not Internal Server Error
    assert!(
        error_msg.contains("already processed") || error_msg.to_lowercase().contains("conflict"),
        "Expected Conflict error, got: {}",
        error_msg
    );
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
