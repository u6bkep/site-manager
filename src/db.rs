use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

pub async fn init(db_path: &str) -> Result<SqlitePool> {
    let url = format!("sqlite:{}?mode=rwc", db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
