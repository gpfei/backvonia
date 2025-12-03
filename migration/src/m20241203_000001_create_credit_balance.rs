use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create user_credit_balance table for persistent credit balances
        manager
            .create_table(
                Table::create()
                    .table(UserCreditBalance::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserCreditBalance::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::PurchaseIdentity)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::SubscriptionCredits)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::SubscriptionMonthlyAllocation)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::SubscriptionResetsAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::ExtraCreditsRemaining)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::LastUpdated)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserCreditBalance::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index on purchase_identity
        manager
            .create_index(
                Index::create()
                    .name("idx_credit_balance_purchase_identity")
                    .table(UserCreditBalance::Table)
                    .col(UserCreditBalance::PurchaseIdentity)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserCreditBalance::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum UserCreditBalance {
    Table,
    Id,
    PurchaseIdentity,
    SubscriptionCredits,
    SubscriptionMonthlyAllocation,
    SubscriptionResetsAt,
    ExtraCreditsRemaining,
    LastUpdated,
    CreatedAt,
}
