/// Application configuration, loaded once at startup from environment variables.
///
/// Add new env vars here rather than calling `std::env::var` elsewhere in the codebase.
pub struct Config {
    pub database_url: String,
    /// Shared secret for `POST /admin/import/scrapes`. `None` means the endpoint is disabled (returns 503).
    pub internal_import_secret: Option<String>,
    /// Maximum number of Postgres connections in the pool. Defaults to 5.
    pub db_max_connections: u32,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            internal_import_secret: std::env::var("RUST_INTERNAL_IMPORT_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            db_max_connections: std::env::var("DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        }
    }
}
