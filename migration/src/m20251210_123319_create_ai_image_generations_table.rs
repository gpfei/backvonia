use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AIImageGeneration::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AIImageGeneration::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AIImageGeneration::UserId).uuid().not_null())
                    // Request context
                    .col(ColumnDef::new(AIImageGeneration::StoryTitle).string().not_null())
                    .col(ColumnDef::new(AIImageGeneration::NodeSummary).string().null())
                    .col(ColumnDef::new(AIImageGeneration::NodeContent).text().null())
                    .col(ColumnDef::new(AIImageGeneration::Style).string().not_null())
                    .col(ColumnDef::new(AIImageGeneration::Resolution).string().not_null())
                    // Generation result
                    .col(ColumnDef::new(AIImageGeneration::ImageUrl).string().not_null())
                    .col(ColumnDef::new(AIImageGeneration::TempUrl).string().null())
                    .col(ColumnDef::new(AIImageGeneration::TempUrlExpiresAt).timestamp_with_time_zone().null())
                    .col(ColumnDef::new(AIImageGeneration::Width).integer().not_null())
                    .col(ColumnDef::new(AIImageGeneration::Height).integer().not_null())
                    .col(ColumnDef::new(AIImageGeneration::FileSizeBytes).integer().null())
                    // Metadata
                    .col(ColumnDef::new(AIImageGeneration::CreditsUsed).integer().not_null().default(10))
                    .col(ColumnDef::new(AIImageGeneration::GenerationTimeMs).integer().null())
                    .col(ColumnDef::new(AIImageGeneration::AiProvider).string().null())
                    .col(ColumnDef::new(AIImageGeneration::Status).string().not_null())
                    .col(ColumnDef::new(AIImageGeneration::ErrorMessage).text().null())
                    // Timestamps
                    .col(
                        ColumnDef::new(AIImageGeneration::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_ai_image_generations_user_id")
                            .from(AIImageGeneration::Table, AIImageGeneration::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on user_id and created_at
        manager
            .create_index(
                Index::create()
                    .name("idx_ai_image_generations_user_created")
                    .table(AIImageGeneration::Table)
                    .col(AIImageGeneration::UserId)
                    .col(AIImageGeneration::CreatedAt)
                    .to_owned(),
            )
            .await?;

        // Create index on status and created_at
        manager
            .create_index(
                Index::create()
                    .name("idx_ai_image_generations_status_created")
                    .table(AIImageGeneration::Table)
                    .col(AIImageGeneration::Status)
                    .col(AIImageGeneration::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AIImageGeneration::Table).to_owned())
            .await
    }
}

// Reference to Users table from first migration
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum AIImageGeneration {
    Table,
    Id,
    UserId,

    // Request context
    StoryTitle,
    NodeSummary,
    NodeContent,
    Style,
    Resolution,

    // Generation result
    ImageUrl,
    TempUrl,
    TempUrlExpiresAt,
    Width,
    Height,
    FileSizeBytes,

    // Metadata
    CreditsUsed,
    GenerationTimeMs,
    AiProvider,
    Status,
    ErrorMessage,

    // Timestamps
    CreatedAt,
}
