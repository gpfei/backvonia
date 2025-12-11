pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_table;
mod m20241201_000001_add_extra_credits;
mod m20241203_000001_create_credit_balance;
mod m20251210_123319_create_ai_image_generations_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_table::Migration),
            Box::new(m20241201_000001_add_extra_credits::Migration),
            Box::new(m20241203_000001_create_credit_balance::Migration),
            Box::new(m20251210_123319_create_ai_image_generations_table::Migration),
        ]
    }
}
