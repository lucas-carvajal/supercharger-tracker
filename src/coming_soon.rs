use serde::{Deserialize, Serialize};

use crate::raw::{ComingSoonDetails, Location};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "site_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SiteStatus {
    InDevelopment,
    UnderConstruction,
    Unknown,
}

impl SiteStatus {
    fn from_opt(s: Option<&str>) -> Self {
        match s {
            Some("In Development") => Self::InDevelopment,
            Some("Under Construction") => Self::UnderConstruction,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for SiteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InDevelopment => write!(f, "In Development"),
            Self::UnderConstruction => write!(f, "Under Construction"),
            Self::Unknown => write!(f, "—"),
        }
    }
}

/// A coming-soon Tesla Supercharger location.
///
/// `id` is the Tesla location URL slug (e.g. `"11255"` from
/// `https://www.tesla.com/findus?location=11255`). It is stable across scrapes
/// and serves as the primary identifier in our system. Tesla's internal UUID
/// is intentionally ignored — it changes arbitrarily for the same location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComingSoonSupercharger {
    /// Stable system identifier — the Tesla location URL slug.
    pub id: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub status: SiteStatus,
    pub raw_status_value: Option<String>,
}

impl ComingSoonSupercharger {
    pub fn is_coming_soon(location: &Location) -> bool {
        location
            .location_type
            .iter()
            .any(|t| t == "coming_soon_supercharger")
    }

    /// Returns the Tesla "Find Us" URL for this location.
    pub fn url(&self) -> String {
        format!("https://www.tesla.com/findus?location={}", self.id)
    }

    /// Apply freshly fetched details to a charger loaded from the DB.
    /// Used by the `retry-failed` command after re-fetching details for failed chargers.
    pub fn with_details(self, details: Option<&ComingSoonDetails>) -> Self {
        let raw_status_value = details.and_then(|d| d.customer_facing_coming_soon_date.clone());
        Self {
            status: SiteStatus::from_opt(raw_status_value.as_deref()),
            raw_status_value,
            ..self
        }
    }

    /// Build a `ComingSoonSupercharger` from a raw API location and its details.
    ///
    /// Returns `None` when the location has no valid slug (empty or `"null"`),
    /// since those entries have no stable identity and cannot be tracked.
    pub fn from_location(l: &Location, details: Option<&ComingSoonDetails>) -> Option<Self> {
        let id = match l.location_url_slug.as_str() {
            "null" | "" => return None,
            s => s.to_string(),
        };
        let raw_status_value = details.and_then(|d| d.customer_facing_coming_soon_date.clone());
        Some(Self {
            id,
            title: l.title.clone(),
            latitude: l.latitude,
            longitude: l.longitude,
            status: SiteStatus::from_opt(raw_status_value.as_deref()),
            raw_status_value,
        })
    }
}
