use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::{
    Decode, Encode, Postgres, Type,
    encode::IsNull,
    error::BoxDynError,
    postgres::{PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueRef},
};

use crate::scraper::raw::{ComingSoonDetails, Location};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
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
///   reappears later, a `Removed → Preliminary` transition is recorded rather than a
///   spurious first-appearance event.
/// - Opened — confirmed open via the `functionTypes=supercharger` endpoint. The row is
///   **copied** to `opened_superchargers` and then **deleted** from this table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SiteStatus {
    /// Earliest build stage (site picked / voted, planning not yet underway).
    Preliminary,
    /// Planning underway.
    Design,
    /// Actively being built.
    Construction,
    /// Details fetch failed or Tesla returned an unrecognised status string.
    Unknown,
    /// Disappeared from the Tesla feed and confirmed absent. Kept as a tombstone row.
    Removed,
    /// Confirmed open via the Tesla `functionTypes=supercharger` endpoint.
    /// Recorded in `status_changes` immediately before the row is deleted from
    /// `coming_soon_superchargers` and copied to `opened_superchargers`.
    Opened,
}

/// Whether a derived status came from Tesla `project_status` or the customer-facing fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDerivationSource {
    ProjectStatus,
    CustomerFacingFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedStatus {
    pub status: SiteStatus,
    pub source: StatusDerivationSource,
}

impl SiteStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Preliminary => "PRELIMINARY",
            Self::Design => "DESIGN",
            Self::Construction => "CONSTRUCTION",
            Self::Unknown => "UNKNOWN",
            Self::Removed => "REMOVED",
            Self::Opened => "OPENED",
        }
    }

    fn pipeline_rank(&self) -> Option<u8> {
        match self {
            Self::Preliminary => Some(0),
            Self::Design => Some(1),
            Self::Construction => Some(2),
            _ => None,
        }
    }
}

impl Type<Postgres> for SiteStatus {
    fn type_info() -> PgTypeInfo {
        <String as Type<Postgres>>::type_info()
    }
}

impl<'r> Decode<'r, Postgres> for SiteStatus {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let s = <&str as Decode<Postgres>>::decode(value)?;
        SiteStatus::from_str(s).map_err(|e| e.into())
    }
}

impl Encode<'_, Postgres> for SiteStatus {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <&str as Encode<Postgres>>::encode_by_ref(&self.as_db_str(), buf)
    }
}

impl PgHasArrayType for SiteStatus {
    fn array_type_info() -> PgTypeInfo {
        <String as PgHasArrayType>::array_type_info()
    }
}

impl FromStr for SiteStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PRELIMINARY" => Ok(Self::Preliminary),
            "DESIGN" => Ok(Self::Design),
            "CONSTRUCTION" => Ok(Self::Construction),
            "UNKNOWN" => Ok(Self::Unknown),
            "REMOVED" => Ok(Self::Removed),
            "OPENED" => Ok(Self::Opened),
            other => Err(format!("unrecognised site status: {other}")),
        }
    }
}

impl std::fmt::Display for SiteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preliminary => write!(f, "Preliminary"),
            Self::Design => write!(f, "Design"),
            Self::Construction => write!(f, "Construction"),
            Self::Unknown => write!(f, "—"),
            Self::Removed => write!(f, "Removed"),
            Self::Opened => write!(f, "Opened"),
        }
    }
}

/// Derive a candidate status from Tesla detail fields (no D8 policy applied).
pub fn derive_status(project_status: Option<&str>, customer_facing: Option<&str>) -> DerivedStatus {
    if let Some(ps) = project_status.filter(|s| !s.is_empty()) {
        match ps {
            "Preliminary" => {
                return DerivedStatus {
                    status: SiteStatus::Preliminary,
                    source: StatusDerivationSource::ProjectStatus,
                };
            }
            "Design" => {
                return DerivedStatus {
                    status: SiteStatus::Design,
                    source: StatusDerivationSource::ProjectStatus,
                };
            }
            "Construction" => {
                return DerivedStatus {
                    status: SiteStatus::Construction,
                    source: StatusDerivationSource::ProjectStatus,
                };
            }
            "Open" => {}
            other => {
                tracing::warn!(
                    value = other,
                    "unrecognised project_status in derive_status — falling back to customer_facing"
                );
            }
        }
    }
    derive_from_customer_facing(customer_facing)
}

fn derive_from_customer_facing(customer_facing: Option<&str>) -> DerivedStatus {
    match customer_facing.filter(|s| !s.is_empty()) {
        Some("In Development") => DerivedStatus {
            status: SiteStatus::Preliminary,
            source: StatusDerivationSource::CustomerFacingFallback,
        },
        Some("Under Construction") => DerivedStatus {
            status: SiteStatus::Construction,
            source: StatusDerivationSource::CustomerFacingFallback,
        },
        Some(other) => {
            tracing::warn!(
                status = other,
                "unrecognised customer_facing status — defaulting to Unknown"
            );
            DerivedStatus {
                status: SiteStatus::Unknown,
                source: StatusDerivationSource::CustomerFacingFallback,
            }
        }
        None => DerivedStatus {
            status: SiteStatus::Unknown,
            source: StatusDerivationSource::CustomerFacingFallback,
        },
    }
}

/// Apply D8 regression policy before persisting a status transition.
pub fn resolve_status_transition(
    existing: SiteStatus,
    candidate: SiteStatus,
    source: StatusDerivationSource,
) -> SiteStatus {
    if existing == SiteStatus::Removed {
        return candidate;
    }

    if candidate == SiteStatus::Unknown {
        return existing;
    }

    let Some(existing_rank) = existing.pipeline_rank() else {
        return candidate;
    };

    let Some(candidate_rank) = candidate.pipeline_rank() else {
        return existing;
    };

    if candidate_rank > existing_rank {
        return candidate;
    }

    if candidate_rank == existing_rank {
        return existing;
    }

    match source {
        StatusDerivationSource::ProjectStatus => {
            tracing::warn!(
                existing = %existing,
                candidate = %candidate,
                "explicit backward project_status transition — recording"
            );
            candidate
        }
        StatusDerivationSource::CustomerFacingFallback => existing,
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

fn status_from_details(details: Option<&ComingSoonDetails>) -> SiteStatus {
    let raw_status_value = details.and_then(|d| d.customer_facing_coming_soon_date.as_deref());
    let raw_project_status = details.and_then(|d| d.project_status.as_deref());
    derive_status(raw_project_status, raw_status_value).status
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
            status: status_from_details(details),
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
            status: status_from_details(details),
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
        assert_eq!(charger.status, SiteStatus::Design);
        assert_eq!(charger.charging_accessibility, None);
        assert_eq!(charger.street_address, None);
    }

    #[test]
    fn derive_status_maps_project_status() {
        assert_eq!(
            derive_status(Some("Design"), Some("In Development")).status,
            SiteStatus::Design
        );
        assert_eq!(
            derive_status(Some("Construction"), None).status,
            SiteStatus::Construction
        );
    }

    #[test]
    fn derive_status_open_falls_back_to_customer_facing() {
        assert_eq!(
            derive_status(Some("Open"), Some("Under Construction")).status,
            SiteStatus::Construction
        );
    }

    #[test]
    fn derive_status_missing_project_status_uses_customer_facing() {
        assert_eq!(
            derive_status(None, Some("In Development")).status,
            SiteStatus::Preliminary
        );
    }

    #[test]
    fn resolve_forward_transition_takes_candidate() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Preliminary,
                SiteStatus::Design,
                StatusDerivationSource::ProjectStatus,
            ),
            SiteStatus::Design
        );
    }

    #[test]
    fn resolve_unknown_candidate_keeps_existing() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Design,
                SiteStatus::Unknown,
                StatusDerivationSource::CustomerFacingFallback,
            ),
            SiteStatus::Design
        );
    }

    #[test]
    fn resolve_fallback_regression_keeps_finer_existing() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Design,
                SiteStatus::Preliminary,
                StatusDerivationSource::CustomerFacingFallback,
            ),
            SiteStatus::Design
        );
    }

    #[test]
    fn resolve_fallback_upgrade_takes_candidate() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Design,
                SiteStatus::Construction,
                StatusDerivationSource::CustomerFacingFallback,
            ),
            SiteStatus::Construction
        );
    }

    #[test]
    fn resolve_explicit_backward_project_status_takes_candidate() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Construction,
                SiteStatus::Design,
                StatusDerivationSource::ProjectStatus,
            ),
            SiteStatus::Design
        );
    }

    #[test]
    fn resolve_removed_reappearance_takes_candidate() {
        assert_eq!(
            resolve_status_transition(
                SiteStatus::Removed,
                SiteStatus::Design,
                StatusDerivationSource::ProjectStatus,
            ),
            SiteStatus::Design
        );
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
