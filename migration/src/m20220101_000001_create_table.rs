use sea_orm_migration::{prelude::*, schema::*};
use sea_orm_migration::sea_query::extension::postgres::Type;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create user_status enum
        manager
            .create_type(
                Type::create()
                    .as_enum(UserStatus::Type)
                    .values([
                        UserStatus::Active,
                        UserStatus::Suspended,
                        UserStatus::Deleted,
                    ])
                    .to_owned(),
            )
            .await?;

        // Create account_tier enum
        manager
            .create_type(
                Type::create()
                    .as_enum(AccountTier::Type)
                    .values([AccountTier::Free, AccountTier::Pro, AccountTier::Enterprise])
                    .to_owned(),
            )
            .await?;

        // Create users table (FIRST - other tables reference this)
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(pk_uuid(Users::Id))
                    .col(string_null(Users::Email).unique_key())
                    .col(boolean(Users::EmailVerified).default(false).not_null())
                    .col(string_null(Users::FullName))
                    .col(
                        ColumnDef::new(Users::Status)
                            .custom(UserStatus::Type)
                            .not_null()
                            .default(SimpleExpr::Custom("'active'::user_status".to_string())),
                    )
                    .col(
                        ColumnDef::new(Users::AccountTier)
                            .custom(AccountTier::Type)
                            .not_null()
                            .default(SimpleExpr::Custom("'free'::account_tier".to_string())),
                    )
                    .col(
                        timestamp_with_time_zone(Users::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(Users::UpdatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(timestamp_with_time_zone_null(Users::LastLoginAt))
                    .to_owned(),
            )
            .await?;

        // Create trigger function for updated_at
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE OR REPLACE FUNCTION update_updated_at_column()
                RETURNS TRIGGER AS $$
                BEGIN
                    NEW.updated_at = NOW();
                    RETURN NEW;
                END;
                $$ LANGUAGE plpgsql;
                "#,
            )
            .await?;

        // Create trigger on users table
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TRIGGER update_users_updated_at
                BEFORE UPDATE ON users
                FOR EACH ROW
                EXECUTE FUNCTION update_updated_at_column();
                "#,
            )
            .await?;

        // Create indexes on users table
        manager
            .create_index(
                Index::create()
                    .name("idx_users_email")
                    .table(Users::Table)
                    .col(Users::Email)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_users_status")
                    .table(Users::Table)
                    .col(Users::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_users_account_tier")
                    .table(Users::Table)
                    .col(Users::AccountTier)
                    .to_owned(),
            )
            .await?;

        // Create user_auth_methods table
        manager
            .create_table(
                Table::create()
                    .table(UserAuthMethods::Table)
                    .if_not_exists()
                    .col(pk_uuid(UserAuthMethods::Id))
                    .col(uuid(UserAuthMethods::UserId).not_null())
                    .col(string(UserAuthMethods::Provider).not_null())
                    .col(string(UserAuthMethods::ProviderUserId).not_null())
                    .col(string_null(UserAuthMethods::ProviderEmail))
                    .col(json_binary_null(UserAuthMethods::ProviderMetadata))
                    .col(
                        timestamp_with_time_zone(UserAuthMethods::FirstLinkedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(UserAuthMethods::LastUsedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_auth_methods_user_id")
                            .from(UserAuthMethods::Table, UserAuthMethods::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index on provider + provider_user_id
        manager
            .create_index(
                Index::create()
                    .name("idx_user_auth_methods_provider_user")
                    .table(UserAuthMethods::Table)
                    .col(UserAuthMethods::Provider)
                    .col(UserAuthMethods::ProviderUserId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create index on user_id
        manager
            .create_index(
                Index::create()
                    .name("idx_user_auth_methods_user_id")
                    .table(UserAuthMethods::Table)
                    .col(UserAuthMethods::UserId)
                    .to_owned(),
            )
            .await?;

        // Create refresh_tokens table
        manager
            .create_table(
                Table::create()
                    .table(RefreshTokens::Table)
                    .if_not_exists()
                    .col(pk_uuid(RefreshTokens::Id))
                    .col(uuid(RefreshTokens::UserId).not_null())
                    .col(string(RefreshTokens::TokenHash).not_null().unique_key())
                    .col(timestamp_with_time_zone(RefreshTokens::ExpiresAt).not_null())
                    .col(
                        timestamp_with_time_zone(RefreshTokens::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(timestamp_with_time_zone_null(RefreshTokens::LastUsedAt))
                    .col(timestamp_with_time_zone_null(RefreshTokens::RevokedAt))
                    .col(json_binary_null(RefreshTokens::DeviceInfo))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_refresh_tokens_user_id")
                            .from(RefreshTokens::Table, RefreshTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on user_id for refresh_tokens
        manager
            .create_index(
                Index::create()
                    .name("idx_refresh_tokens_user_id")
                    .table(RefreshTokens::Table)
                    .col(RefreshTokens::UserId)
                    .to_owned(),
            )
            .await?;

        // Create index on token_hash for fast lookup
        manager
            .create_index(
                Index::create()
                    .name("idx_refresh_tokens_token_hash")
                    .table(RefreshTokens::Table)
                    .col(RefreshTokens::TokenHash)
                    .to_owned(),
            )
            .await?;

        // Create index on expires_at for cleanup queries
        manager
            .create_index(
                Index::create()
                    .name("idx_refresh_tokens_expires_at")
                    .table(RefreshTokens::Table)
                    .col(RefreshTokens::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // Create user_iap_receipts table
        manager
            .create_table(
                Table::create()
                    .table(UserIapReceipts::Table)
                    .if_not_exists()
                    .col(pk_uuid(UserIapReceipts::Id))
                    .col(uuid(UserIapReceipts::UserId).not_null())
                    .col(string(UserIapReceipts::OriginalTransactionId).not_null())
                    .col(string(UserIapReceipts::Platform).not_null())
                    .col(boolean(UserIapReceipts::IsFamilyShared).default(false).not_null())
                    .col(uuid_null(UserIapReceipts::FamilyPrimaryUserId))
                    .col(string(UserIapReceipts::ProductId).not_null())
                    .col(
                        ColumnDef::new(UserIapReceipts::PurchaseTier)
                            .custom(AccountTier::Type)
                            .not_null(),
                    )
                    .col(string_null(UserIapReceipts::SubscriptionStatus))
                    .col(timestamp_with_time_zone_null(UserIapReceipts::ExpiresAt))
                    .col(string(UserIapReceipts::ReceiptHash).not_null())
                    .col(timestamp_with_time_zone(UserIapReceipts::LastVerifiedAt).not_null())
                    .col(
                        timestamp_with_time_zone(UserIapReceipts::FirstLinkedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(UserIapReceipts::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(UserIapReceipts::UpdatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_iap_receipts_user_id")
                            .from(UserIapReceipts::Table, UserIapReceipts::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_iap_receipts_family_primary_user_id")
                            .from(UserIapReceipts::Table, UserIapReceipts::FamilyPrimaryUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Create trigger on user_iap_receipts table
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TRIGGER update_user_iap_receipts_updated_at
                BEFORE UPDATE ON user_iap_receipts
                FOR EACH ROW
                EXECUTE FUNCTION update_updated_at_column();
                "#,
            )
            .await?;

        // Create indexes on user_iap_receipts
        manager
            .create_index(
                Index::create()
                    .name("idx_user_iap_receipts_user_id")
                    .table(UserIapReceipts::Table)
                    .col(UserIapReceipts::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_iap_receipts_original_txn")
                    .table(UserIapReceipts::Table)
                    .col(UserIapReceipts::OriginalTransactionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_iap_receipts_family_primary")
                    .table(UserIapReceipts::Table)
                    .col(UserIapReceipts::FamilyPrimaryUserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_iap_receipts_status")
                    .table(UserIapReceipts::Table)
                    .col(UserIapReceipts::SubscriptionStatus)
                    .to_owned(),
            )
            .await?;

        // Create unique index on user_id + original_transaction_id
        manager
            .create_index(
                Index::create()
                    .name("idx_user_iap_receipts_user_txn")
                    .table(UserIapReceipts::Table)
                    .col(UserIapReceipts::UserId)
                    .col(UserIapReceipts::OriginalTransactionId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create quota_usage table (with user_id FK)
        manager
            .create_table(
                Table::create()
                    .table(QuotaUsage::Table)
                    .if_not_exists()
                    .col(pk_uuid(QuotaUsage::Id))
                    .col(uuid(QuotaUsage::UserId).not_null())
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
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_quota_usage_user_id")
                            .from(QuotaUsage::Table, QuotaUsage::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index on quota_usage (user_id + usage_date)
        manager
            .create_index(
                Index::create()
                    .name("idx_quota_usage_user_date")
                    .table(QuotaUsage::Table)
                    .col(QuotaUsage::UserId)
                    .col(QuotaUsage::UsageDate)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create iap_receipt_cache table (with user_id FK)
        manager
            .create_table(
                Table::create()
                    .table(IapReceiptCache::Table)
                    .if_not_exists()
                    .col(pk_uuid(IapReceiptCache::Id))
                    .col(uuid_null(IapReceiptCache::UserId))
                    .col(string(IapReceiptCache::OriginalTransactionId).unique_key().not_null())
                    .col(string(IapReceiptCache::Platform).not_null())
                    .col(
                        ColumnDef::new(IapReceiptCache::PurchaseTier)
                            .custom(AccountTier::Type)
                            .not_null(),
                    )
                    .col(string_null(IapReceiptCache::ProductId))
                    .col(string_null(IapReceiptCache::ReceiptHash))
                    .col(timestamp_with_time_zone_null(IapReceiptCache::ValidUntil))
                    .col(timestamp_with_time_zone(IapReceiptCache::LastVerifiedAt).not_null())
                    .col(
                        timestamp_with_time_zone(IapReceiptCache::CreatedAt)
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_iap_receipt_cache_user_id")
                            .from(IapReceiptCache::Table, IapReceiptCache::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on iap_receipt_cache user_id
        manager
            .create_index(
                Index::create()
                    .name("idx_iap_receipt_cache_user_id")
                    .table(IapReceiptCache::Table)
                    .col(IapReceiptCache::UserId)
                    .to_owned(),
            )
            .await?;

        // Create index on iap_receipt_cache original_transaction_id
        manager
            .create_index(
                Index::create()
                    .name("idx_iap_receipt_cache_original_txn")
                    .table(IapReceiptCache::Table)
                    .col(IapReceiptCache::OriginalTransactionId)
                    .to_owned(),
            )
            .await?;

        // Create index on iap_receipt_cache receipt_hash
        manager
            .create_index(
                Index::create()
                    .name("idx_iap_receipt_cache_hash")
                    .table(IapReceiptCache::Table)
                    .col(IapReceiptCache::ReceiptHash)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse order (due to foreign keys)
        manager
            .drop_table(Table::drop().table(IapReceiptCache::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(QuotaUsage::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(UserIapReceipts::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(RefreshTokens::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(UserAuthMethods::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await?;

        // Drop trigger function
        manager
            .get_connection()
            .execute_unprepared("DROP FUNCTION IF EXISTS update_updated_at_column() CASCADE;")
            .await?;

        // Drop enums
        manager
            .drop_type(Type::drop().name(AccountTier::Type).to_owned())
            .await?;

        manager
            .drop_type(Type::drop().name(UserStatus::Type).to_owned())
            .await?;

        Ok(())
    }
}

// Enum definitions
#[derive(DeriveIden)]
enum UserStatus {
    #[sea_orm(iden = "user_status")]
    Type,
    Active,
    Suspended,
    Deleted,
}

#[derive(DeriveIden)]
enum AccountTier {
    #[sea_orm(iden = "account_tier")]
    Type,
    Free,
    Pro,
    Enterprise,
}

// Table definitions
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    EmailVerified,
    FullName,
    Status,
    AccountTier,
    CreatedAt,
    UpdatedAt,
    LastLoginAt,
}

#[derive(DeriveIden)]
enum UserAuthMethods {
    Table,
    Id,
    UserId,
    Provider,
    ProviderUserId,
    ProviderEmail,
    ProviderMetadata,
    FirstLinkedAt,
    LastUsedAt,
}

#[derive(DeriveIden)]
enum RefreshTokens {
    Table,
    Id,
    UserId,
    TokenHash,
    ExpiresAt,
    CreatedAt,
    LastUsedAt,
    RevokedAt,
    DeviceInfo,
}

#[derive(DeriveIden)]
enum UserIapReceipts {
    Table,
    Id,
    UserId,
    OriginalTransactionId,
    Platform,
    IsFamilyShared,
    FamilyPrimaryUserId,
    ProductId,
    PurchaseTier,
    SubscriptionStatus,
    ExpiresAt,
    ReceiptHash,
    LastVerifiedAt,
    FirstLinkedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum QuotaUsage {
    Table,
    Id,
    UserId,
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
    UserId,
    OriginalTransactionId,
    Platform,
    PurchaseTier,
    ProductId,
    ReceiptHash,
    ValidUntil,
    LastVerifiedAt,
    CreatedAt,
}
