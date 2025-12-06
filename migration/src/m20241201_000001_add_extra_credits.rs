use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create credit_purchases table
        manager
            .create_table(
                Table::create()
                    .table(CreditPurchases::Table)
                    .if_not_exists()
                    .col(pk_uuid(CreditPurchases::Id))
                    .col(uuid(CreditPurchases::UserId).not_null())
                    .col(string_null(CreditPurchases::OriginalTransactionId))
                    .col(
                        string(CreditPurchases::TransactionId)
                            .unique_key()
                            .not_null(),
                    )
                    .col(string(CreditPurchases::ProductId).not_null())
                    .col(string(CreditPurchases::Platform).not_null())
                    .col(integer(CreditPurchases::Amount).not_null())
                    .col(integer(CreditPurchases::Consumed).default(0).not_null())
                    .col(timestamp_with_time_zone(CreditPurchases::PurchaseDate).not_null())
                    .col(
                        timestamp_with_time_zone(CreditPurchases::VerifiedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(text_null(CreditPurchases::ReceiptData))
                    .col(timestamp_with_time_zone_null(CreditPurchases::RevokedAt))
                    .col(string_null(CreditPurchases::RevokedReason))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_credit_purchases_user_id")
                            .from(CreditPurchases::Table, CreditPurchases::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create indexes on credit_purchases
        manager
            .create_index(
                Index::create()
                    .name("idx_credit_purchases_user_id")
                    .table(CreditPurchases::Table)
                    .col(CreditPurchases::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_credit_purchases_transaction_id")
                    .table(CreditPurchases::Table)
                    .col(CreditPurchases::TransactionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_credit_purchases_original_transaction_id")
                    .table(CreditPurchases::Table)
                    .col(CreditPurchases::OriginalTransactionId)
                    .to_owned(),
            )
            .await?;

        // FIFO index for consumption order
        manager
            .create_index(
                Index::create()
                    .name("idx_credit_purchases_user_purchase_date")
                    .table(CreditPurchases::Table)
                    .col(CreditPurchases::UserId)
                    .col(CreditPurchases::PurchaseDate)
                    .to_owned(),
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
        // Drop credit_purchases table
        manager
            .drop_table(Table::drop().table(CreditPurchases::Table).to_owned())
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
enum CreditPurchases {
    Table,
    Id,
    UserId, // Changed from LocalUserId
    OriginalTransactionId,
    TransactionId,
    ProductId,
    Platform,
    Amount,
    Consumed,
    PurchaseDate,
    VerifiedAt,
    ReceiptData,
    RevokedAt,
    RevokedReason,
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
