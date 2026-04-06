/// Application configuration, loaded once at startup from environment variables.
///
/// Add new env vars here rather than calling `std::env::var` elsewhere in the codebase.
pub struct Config {
    pub database_url: String,
    /// Shared secret for `POST /scrapes/import`. `None` means the endpoint is disabled (returns 503).
    pub import_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            import_token: std::env::var("IMPORT_TOKEN").ok().filter(|s| !s.is_empty()),
        }
    }
}
