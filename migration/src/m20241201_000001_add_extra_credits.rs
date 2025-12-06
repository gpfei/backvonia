use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create credits_events table (unified ledger for purchases, welcome bonuses, adjustments)
        manager
            .create_table(
                Table::create()
                    .table(CreditsEvents::Table)
                    .if_not_exists()
                    .col(pk_uuid(CreditsEvents::Id))
                    .col(uuid(CreditsEvents::UserId).not_null())
                    .col(string(CreditsEvents::EventType).not_null())
                    .col(string_null(CreditsEvents::OriginalTransactionId))
                    .col(string(CreditsEvents::TransactionId).unique_key().not_null())
                    .col(string_null(CreditsEvents::ProductId))
                    .col(string_null(CreditsEvents::Platform))
                    .col(integer(CreditsEvents::Amount).not_null())
                    .col(integer(CreditsEvents::Consumed).default(0).not_null())
                    .col(timestamp_with_time_zone(CreditsEvents::OccurredAt).not_null())
                    .col(
                        timestamp_with_time_zone(CreditsEvents::VerifiedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(text_null(CreditsEvents::ReceiptData))
                    .col(timestamp_with_time_zone_null(CreditsEvents::RevokedAt))
                    .col(string_null(CreditsEvents::RevokedReason))
                    .col(string_null(CreditsEvents::DeviceId))
                    .col(string_null(CreditsEvents::Provider))
                    .col(string_null(CreditsEvents::ProviderUserId))
                    .col(json_binary_null(CreditsEvents::Metadata))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_credits_events_user_id")
                            .from(CreditsEvents::Table, CreditsEvents::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create indexes on credits_events
        manager
            .create_index(
                Index::create()
                    .name("idx_credits_events_user_id")
                    .table(CreditsEvents::Table)
                    .col(CreditsEvents::UserId)
                    .col(CreditsEvents::OccurredAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_credits_events_transaction_id")
                    .table(CreditsEvents::Table)
                    .col(CreditsEvents::TransactionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_credits_events_original_transaction_id")
                    .table(CreditsEvents::Table)
                    .col(CreditsEvents::OriginalTransactionId)
                    .to_owned(),
            )
            .await?;

        // Partial uniqueness for welcome bonuses (one per user/device)
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE UNIQUE INDEX IF NOT EXISTS idx_credits_events_welcome_user
                ON credits_events (event_type, user_id)
                WHERE event_type = 'welcome_bonus';
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE UNIQUE INDEX IF NOT EXISTS idx_credits_events_welcome_device
                ON credits_events (event_type, device_id)
                WHERE event_type = 'welcome_bonus' AND device_id IS NOT NULL;
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE INDEX IF NOT EXISTS idx_credits_events_welcome_provider
                ON credits_events (provider, provider_user_id)
                WHERE event_type = 'welcome_bonus';
                "#,
            )
            .await?;

        // Add extra credits columns to quota_usage table
        manager
            .alter_table(
                Table::alter()
                    .table(QuotaUsage::Table)
                    .add_column_if_not_exists(
                        integer(QuotaUsage::ExtraCreditsTotal).default(0).not_null(),
                    )
                    .add_column_if_not_exists(
                        integer(QuotaUsage::SubscriptionCredits)
                            .default(0)
                            .not_null(),
                    )
                    .add_column_if_not_exists(
                        integer(QuotaUsage::SubscriptionMonthlyAllocation)
                            .default(0)
                            .not_null(),
                    )
                    .add_column_if_not_exists(timestamp_with_time_zone_null(
                        QuotaUsage::LastExtraCreditsSync,
                    ))
                    .add_column_if_not_exists(timestamp_with_time_zone_null(
                        QuotaUsage::SubscriptionResetsAt,
                    ))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop partial indexes
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DROP INDEX IF EXISTS idx_credits_events_welcome_user;
                DROP INDEX IF EXISTS idx_credits_events_welcome_device;
                DROP INDEX IF EXISTS idx_credits_events_welcome_provider;
                "#,
            )
            .await?;

        // Drop credits_events table
        manager
            .drop_table(Table::drop().table(CreditsEvents::Table).to_owned())
            .await?;

        // Remove extra credits columns from quota_usage
        manager
            .alter_table(
                Table::alter()
                    .table(QuotaUsage::Table)
                    .drop_column(QuotaUsage::ExtraCreditsTotal)
                    .drop_column(QuotaUsage::SubscriptionCredits)
                    .drop_column(QuotaUsage::SubscriptionMonthlyAllocation)
                    .drop_column(QuotaUsage::LastExtraCreditsSync)
                    .drop_column(QuotaUsage::SubscriptionResetsAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

// Reference to Users table from first migration
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum CreditsEvents {
    Table,
    Id,
    UserId, // Changed from LocalUserId
    EventType,
    OriginalTransactionId,
    TransactionId,
    ProductId,
    Platform,
    Amount,
    Consumed,
    OccurredAt,
    VerifiedAt,
    ReceiptData,
    RevokedAt,
    RevokedReason,
    DeviceId,
    Provider,
    ProviderUserId,
    Metadata,
}

#[derive(DeriveIden)]
enum QuotaUsage {
    Table,
    ExtraCreditsTotal,
    SubscriptionCredits,
    SubscriptionMonthlyAllocation,
    LastExtraCreditsSync,
    SubscriptionResetsAt,
}
