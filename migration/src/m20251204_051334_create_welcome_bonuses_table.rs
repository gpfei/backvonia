use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create welcome_bonuses table
        manager
            .create_table(
                Table::create()
                    .table(WelcomeBonuses::Table)
                    .if_not_exists()
                    .col(pk_uuid(WelcomeBonuses::Id))
                    .col(uuid(WelcomeBonuses::UserId).not_null())
                    .col(string(WelcomeBonuses::DeviceId).not_null())
                    .col(string(WelcomeBonuses::Provider).not_null())
                    .col(string(WelcomeBonuses::ProviderUserId).not_null())
                    .col(integer(WelcomeBonuses::AmountGranted).not_null())
                    .col(
                        string(WelcomeBonuses::Reason)
                            .not_null()
                            .default("new_user"),
                    )
                    .col(
                        timestamp_with_time_zone(WelcomeBonuses::GrantedAt)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_welcome_bonuses_user_id")
                            .from(WelcomeBonuses::Table, WelcomeBonuses::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Add unique constraint on user_id (one bonus per user)
        manager
            .create_index(
                Index::create()
                    .name("idx_welcome_bonuses_user_id")
                    .table(WelcomeBonuses::Table)
                    .col(WelcomeBonuses::UserId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Add unique constraint on device_id (one bonus per device)
        manager
            .create_index(
                Index::create()
                    .name("idx_welcome_bonuses_device_id")
                    .table(WelcomeBonuses::Table)
                    .col(WelcomeBonuses::DeviceId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Add index on provider and provider_user_id for fraud checks
        manager
            .create_index(
                Index::create()
                    .name("idx_welcome_bonuses_provider_user")
                    .table(WelcomeBonuses::Table)
                    .col(WelcomeBonuses::Provider)
                    .col(WelcomeBonuses::ProviderUserId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(WelcomeBonuses::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum WelcomeBonuses {
    Table,
    Id,
    UserId,
    DeviceId,
    Provider,
    ProviderUserId,
    AmountGranted,
    Reason,
    GrantedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
