use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
