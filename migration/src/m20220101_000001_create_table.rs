use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create quota_usage table
        manager
            .create_table(
                Table::create()
                    .table(QuotaUsage::Table)
                    .if_not_exists()
                    .col(pk_uuid(QuotaUsage::Id))
                    .col(string(QuotaUsage::PurchaseIdentity).not_null())
                    .col(date(QuotaUsage::UsageDate).not_null())
                    .col(integer(QuotaUsage::TextCount).default(0).not_null())
                    .col(integer(QuotaUsage::ImageCount).default(0).not_null())
                    .col(
                        timestamp_with_time_zone(QuotaUsage::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(QuotaUsage::UpdatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index on quota_usage
        manager
            .create_index(
                Index::create()
                    .name("idx_quota_usage_identity_date")
                    .table(QuotaUsage::Table)
                    .col(QuotaUsage::PurchaseIdentity)
                    .col(QuotaUsage::UsageDate)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create iap_receipt_cache table
        manager
            .create_table(
                Table::create()
                    .table(IapReceiptCache::Table)
                    .if_not_exists()
                    .col(pk_uuid(IapReceiptCache::Id))
                    .col(
                        string(IapReceiptCache::PurchaseIdentity)
                            .unique_key()
                            .not_null(),
                    )
                    .col(string(IapReceiptCache::Platform).not_null())
                    .col(string(IapReceiptCache::PurchaseTier).not_null())
                    .col(string_null(IapReceiptCache::ProductId))
                    .col(string_null(IapReceiptCache::ReceiptHash))
                    .col(timestamp_with_time_zone_null(IapReceiptCache::ValidUntil))
                    .col(timestamp_with_time_zone(IapReceiptCache::LastVerifiedAt).not_null())
                    .col(
                        timestamp_with_time_zone(IapReceiptCache::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on iap_receipt_cache purchase_identity
        manager
            .create_index(
                Index::create()
                    .name("idx_iap_receipt_purchase_identity")
                    .table(IapReceiptCache::Table)
                    .col(IapReceiptCache::PurchaseIdentity)
                    .to_owned(),
            )
            .await?;

        // Create index on iap_receipt_cache receipt_hash
        manager
            .create_index(
                Index::create()
                    .name("idx_iap_receipt_hash")
                    .table(IapReceiptCache::Table)
                    .col(IapReceiptCache::ReceiptHash)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(IapReceiptCache::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(QuotaUsage::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum QuotaUsage {
    Table,
    Id,
    PurchaseIdentity,
    UsageDate,
    TextCount,
    ImageCount,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum IapReceiptCache {
    Table,
    Id,
    PurchaseIdentity,
    Platform,
    PurchaseTier,
    ProductId,
    ReceiptHash,
    ValidUntil,
    LastVerifiedAt,
    CreatedAt,
}
