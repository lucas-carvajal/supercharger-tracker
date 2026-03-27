use serde::{Deserialize, Serialize};

use crate::raw::{ComingSoonDetails, Location};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComingSoonStatus {
    InDevelopment,
    UnderConstruction,
    Unknown,
}

impl ComingSoonStatus {
    fn from_opt(s: Option<&str>) -> Self {
        match s {
            Some("In Development") => Self::InDevelopment,
            Some("Under Construction") => Self::UnderConstruction,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for ComingSoonStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InDevelopment => write!(f, "In Development"),
            Self::UnderConstruction => write!(f, "Under Construction"),
            Self::Unknown => write!(f, "—"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComingSoonSupercharger {
    pub uuid: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub status: ComingSoonStatus,
    pub location_url_slug: Option<String>,
}

impl ComingSoonSupercharger {
    pub fn is_coming_soon(location: &Location) -> bool {
        location
            .location_type
            .iter()
            .any(|t| t == "coming_soon_supercharger")
    }

    pub fn url(&self) -> Option<String> {
        self.location_url_slug
            .as_deref()
            .map(|slug| format!("https://www.tesla.com/findus?location={slug}"))
    }

    pub fn from_location(l: &Location, details: Option<&ComingSoonDetails>) -> Self {
        let slug = match l.location_url_slug.as_str() {
            "null" | "" => None,
            s => Some(s.to_string()),
        };
        Self {
            uuid: l.uuid.clone(),
            title: l.title.clone(),
            latitude: l.latitude,
            longitude: l.longitude,
            status: ComingSoonStatus::from_opt(
                details.and_then(|d| d.customer_facing_coming_soon_date.as_deref()),
            ),
            location_url_slug: slug,
        }
    }
}
