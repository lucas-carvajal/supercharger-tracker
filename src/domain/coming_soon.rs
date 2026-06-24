use serde::{Deserialize, Serialize};

use crate::scraper::raw::{ComingSoonDetails, Location};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "charger_category", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChargerCategory {
    ComingSoon,
    Winner,
    CurrentWinner,
}

fn category_from_location(location: &Location) -> ChargerCategory {
    if location
        .location_type
        .iter()
        .any(|t| t == "current_winner_supercharger")
    {
        ChargerCategory::CurrentWinner
    } else if location
        .location_type
        .iter()
        .any(|t| t == "winner_supercharger")
    {
        ChargerCategory::Winner
    } else {
        ChargerCategory::ComingSoon
    }
}

/// Status of a coming-soon Supercharger location.
///
/// Active chargers live in `coming_soon_superchargers` with one of the first three variants.
/// A charger leaves that table in one of two ways:
/// - [`Removed`](SiteStatus::Removed) — disappeared from the Tesla feed and confirmed absent
///   via the open-status check. The row is **kept** as a tombstone so that if the location
///   reappears later, a `Removed → InDevelopment` transition is recorded rather than a
///   spurious first-appearance event.
/// - Opened — confirmed open via the `functionTypes=supercharger` endpoint. The row is
///   **copied** to `opened_superchargers` and then **deleted** from this table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "site_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SiteStatus {
    /// Planned but not yet under active construction.
    InDevelopment,
    /// Actively being built.
    UnderConstruction,
    /// Details fetch failed or Tesla returned an unrecognised status string.
    Unknown,
    /// Disappeared from the Tesla feed and confirmed absent. Kept as a tombstone row.
    Removed,
    /// Confirmed open via the Tesla `functionTypes=supercharger` endpoint.
    /// Recorded in `status_changes` immediately before the row is deleted from
    /// `coming_soon_superchargers` and copied to `opened_superchargers`.
    Opened,
}

impl SiteStatus {
    fn from_opt(s: Option<&str>) -> Self {
        match s {
            Some("In Development") => Self::InDevelopment,
            Some("Under Construction") => Self::UnderConstruction,
            Some(other) => {
                tracing::warn!(
                    status = other,
                    "unrecognised site status — defaulting to Unknown"
                );
                Self::Unknown
            }
            None => Self::Unknown,
        }
    }
}

impl std::fmt::Display for SiteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InDevelopment => write!(f, "In Development"),
            Self::UnderConstruction => write!(f, "Under Construction"),
            Self::Unknown => write!(f, "—"),
            Self::Removed => write!(f, "Removed"),
            Self::Opened => write!(f, "Opened"),
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
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: SiteStatus,
    pub raw_status_value: Option<String>,
    pub raw_project_status: Option<String>,
    /// `0` means unknown / not yet published by Tesla.
    pub num_charger_stalls: i32,
    pub charging_accessibility: Option<String>,
    pub street_address: Option<String>,
    pub county: Option<String>,
    pub postal_code: Option<String>,
    pub country_code: Option<String>,
    pub charger_category: ChargerCategory,
}

/// Parse stall count from Tesla's string field. Missing or unparseable → `0` (unknown).
pub fn parse_num_charger_stalls(raw: Option<&str>) -> i32 {
    raw.and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
}

fn detail_fields_from(details: Option<&ComingSoonDetails>) -> DetailFieldValues {
    DetailFieldValues {
        raw_project_status: details.and_then(|d| non_empty_string(d.project_status.clone())),
        num_charger_stalls: parse_num_charger_stalls(
            details.and_then(|d| d.num_charger_stalls.as_deref()),
        ),
        charging_accessibility: details
            .and_then(|d| non_empty_string(d.charging_accessibility.clone())),
        street_address: details.and_then(|d| non_empty_string(d.street_address.clone())),
        county: details.and_then(|d| non_empty_string(d.county.clone())),
        postal_code: details.and_then(|d| non_empty_string(d.postal_code.clone())),
        country_code: details.and_then(|d| non_empty_string(d.country_code.clone())),
    }
}

struct DetailFieldValues {
    raw_project_status: Option<String>,
    num_charger_stalls: i32,
    charging_accessibility: Option<String>,
    street_address: Option<String>,
    county: Option<String>,
    postal_code: Option<String>,
    country_code: Option<String>,
}

/// Splits `"City, Region"` on the last comma, trims both sides.
/// Returns `(None, None)` if there is no comma or either side is empty after trimming.
fn parse_title(title: &str) -> (Option<String>, Option<String>) {
    let Some(comma) = title.rfind(',') else {
        return (None, None);
    };
    let city = title[..comma].trim().to_string();
    let region = title[comma + 1..].trim().to_string();
    if city.is_empty() || region.is_empty() {
        return (None, None);
    }
    (Some(city), Some(region))
}

impl ComingSoonSupercharger {
    pub fn is_coming_soon(location: &Location) -> bool {
        location.location_type.iter().any(|t| {
            matches!(
                t.as_str(),
                "coming_soon_supercharger" | "winner_supercharger" | "current_winner_supercharger"
            )
        })
    }

    /// Returns the Tesla "Find Us" URL for this location.
    pub fn url(&self) -> String {
        format!("https://www.tesla.com/findus?location={}", self.id)
    }

    /// Apply freshly fetched details to a charger loaded from the DB.
    /// Used by the `retry-failed` command after re-fetching details for failed chargers.
    pub fn with_details(self, details: Option<&ComingSoonDetails>) -> Self {
        let raw_status_value = details.and_then(|d| d.customer_facing_coming_soon_date.clone());
        let title = details
            .and_then(|d| d.coming_soon_name.clone())
            .unwrap_or(self.title.clone());
        let (city, region) = parse_title(&title);
        let detail_fields = detail_fields_from(details);
        Self {
            status: SiteStatus::from_opt(raw_status_value.as_deref()),
            raw_status_value,
            raw_project_status: detail_fields.raw_project_status,
            num_charger_stalls: detail_fields.num_charger_stalls,
            charging_accessibility: detail_fields.charging_accessibility,
            street_address: detail_fields.street_address,
            county: detail_fields.county,
            postal_code: detail_fields.postal_code,
            country_code: detail_fields.country_code,
            title,
            city,
            region,
            ..self // charger_category passes through unchanged
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
        let title = details
            .and_then(|d| d.coming_soon_name.clone())
            .unwrap_or_else(|| l.title.clone());
        let (city, region) = parse_title(&title);
        let detail_fields = detail_fields_from(details);
        Some(Self {
            id,
            title,
            city,
            region,
            latitude: l.latitude,
            longitude: l.longitude,
            status: SiteStatus::from_opt(raw_status_value.as_deref()),
            raw_status_value,
            raw_project_status: detail_fields.raw_project_status,
            num_charger_stalls: detail_fields.num_charger_stalls,
            charging_accessibility: detail_fields.charging_accessibility,
            street_address: detail_fields.street_address,
            county: detail_fields.county,
            postal_code: detail_fields.postal_code,
            country_code: detail_fields.country_code,
            charger_category: category_from_location(l),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scraper::raw::{ComingSoonDetails, Location};

    #[test]
    fn parse_num_charger_stalls_maps_values() {
        assert_eq!(parse_num_charger_stalls(Some("8")), 8);
        assert_eq!(parse_num_charger_stalls(Some("0")), 0);
        assert_eq!(parse_num_charger_stalls(None), 0);
        assert_eq!(parse_num_charger_stalls(Some("not-a-number")), 0);
    }

    #[test]
    fn from_location_treats_empty_strings_as_unknown() {
        let location = Location {
            uuid: "u".into(),
            title: "City, Region".into(),
            latitude: 1.0,
            longitude: 2.0,
            location_type: vec!["coming_soon_supercharger".into()],
            location_url_slug: "123".into(),
            supercharger_function: None,
        };
        let details = ComingSoonDetails {
            project_status: Some("Design".into()),
            charging_accessibility: Some("".into()),
            street_address: Some("".into()),
            ..Default::default()
        };

        let charger = ComingSoonSupercharger::from_location(&location, Some(&details)).unwrap();

        assert_eq!(charger.raw_project_status.as_deref(), Some("Design"));
        assert_eq!(charger.charging_accessibility, None);
        assert_eq!(charger.street_address, None);
    }

    fn rule_a_text<'a>(existing: Option<&'a str>, incoming: Option<&'a str>) -> Option<&'a str> {
        match incoming {
            None | Some("") => existing,
            Some(v) => Some(v),
        }
    }

    fn rule_a_stalls(existing: i32, incoming: i32) -> i32 {
        if incoming == 0 { existing } else { incoming }
    }

    #[test]
    fn rule_a_text_keeps_existing_when_incoming_unknown() {
        assert_eq!(rule_a_text(Some("Design"), None), Some("Design"));
        assert_eq!(rule_a_text(Some("Design"), Some("")), Some("Design"));
        assert_eq!(
            rule_a_text(Some("Design"), Some("Construction")),
            Some("Construction")
        );
        assert_eq!(rule_a_text(None, Some("Design")), Some("Design"));
    }

    #[test]
    fn rule_a_stalls_keeps_existing_when_incoming_unknown() {
        assert_eq!(rule_a_stalls(8, 0), 8);
        assert_eq!(rule_a_stalls(8, 12), 12);
        assert_eq!(rule_a_stalls(0, 4), 4);
    }
}
