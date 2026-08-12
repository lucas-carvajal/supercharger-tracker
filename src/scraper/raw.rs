use serde::Deserialize;
use serde_json::Value;

/// Tesla sometimes emits JSON `null` for string fields (slug, title, uuid) or
/// for individual `location_type` entries. Treat those as empty / omit them so
/// one bad row cannot abort an entire scrape of thousands of locations.
#[derive(Deserialize)]
pub struct Location {
    #[serde(default, deserialize_with = "null_as_empty_string")]
    #[allow(dead_code)]
    pub uuid: String,
    #[serde(default, deserialize_with = "null_as_empty_string")]
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default, deserialize_with = "null_filtered_string_vec")]
    pub location_type: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty_string")]
    pub location_url_slug: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub supercharger_function: Option<SuperchargerFunction>,
}

fn null_as_empty_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn null_filtered_string_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<Option<String>>>::deserialize(deserializer)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect())
}

/// Parse the get-locations payload, skipping individual rows that still fail
/// deserialization (e.g. non-numeric coordinates). Logs each skipped row.
pub fn parse_locations_response(
    json_text: &str,
) -> Result<Vec<Location>, Box<dyn std::error::Error>> {
    let root: Value = serde_json::from_str(json_text)?;
    let Some(items) = root
        .get("data")
        .and_then(|d| d.get("data"))
        .and_then(|d| d.as_array())
    else {
        return Err("API response missing data.data array".into());
    };

    let mut locations = Vec::with_capacity(items.len());
    let mut skipped = 0usize;

    for (index, item) in items.iter().enumerate() {
        match serde_json::from_value::<Location>(item.clone()) {
            Ok(location) => locations.push(location),
            Err(err) => {
                skipped += 1;
                let slug = item
                    .get("location_url_slug")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<missing>");
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<missing>");
                let null_string_fields: Vec<&str> = [
                    "uuid",
                    "title",
                    "location_url_slug",
                    "latitude",
                    "longitude",
                    "location_type",
                ]
                .into_iter()
                .filter(|field| item.get(*field).is_some_and(|v| v.is_null()))
                .collect();
                tracing::warn!(
                    index,
                    slug,
                    title,
                    error = %err,
                    null_fields = ?null_string_fields,
                    "skipping location that failed to deserialize"
                );
            }
        }
    }

    if locations.is_empty() && !items.is_empty() {
        return Err(format!(
            "failed to deserialize any of {} location entries",
            items.len()
        )
        .into());
    }

    if skipped > 0 {
        tracing::warn!(
            skipped,
            kept = locations.len(),
            total = items.len(),
            "some locations were skipped due to schema mismatches"
        );
    }

    Ok(locations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_accepts_null_string_fields() {
        let json = r#"{
            "uuid": null,
            "title": null,
            "latitude": 1.0,
            "longitude": 2.0,
            "location_type": ["coming_soon_supercharger", null, ""],
            "location_url_slug": null
        }"#;
        let loc: Location = serde_json::from_str(json).unwrap();
        assert_eq!(loc.uuid, "");
        assert_eq!(loc.title, "");
        assert_eq!(loc.location_url_slug, "");
        assert_eq!(loc.location_type, vec!["coming_soon_supercharger"]);
    }

    #[test]
    fn parse_locations_response_skips_bad_rows() {
        let json = r#"{
            "data": {
                "data": [
                    {
                        "uuid": "1",
                        "title": "Good, Place",
                        "latitude": 1.0,
                        "longitude": 2.0,
                        "location_type": ["coming_soon_supercharger"],
                        "location_url_slug": "abc"
                    },
                    {
                        "uuid": "2",
                        "title": "Bad coords",
                        "latitude": "not-a-number",
                        "longitude": 2.0,
                        "location_type": ["coming_soon_supercharger"],
                        "location_url_slug": "def"
                    },
                    {
                        "uuid": null,
                        "title": null,
                        "latitude": 3.0,
                        "longitude": 4.0,
                        "location_type": null,
                        "location_url_slug": "ghi"
                    }
                ]
            }
        }"#;

        let locations = parse_locations_response(json).unwrap();
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].location_url_slug, "abc");
        assert_eq!(locations[1].location_url_slug, "ghi");
        assert!(locations[1].title.is_empty());
        assert!(locations[1].location_type.is_empty());
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct SuperchargerFunction {
    pub access_type: Option<String>,
    pub open_to_non_tesla: Option<bool>,
    pub site_status: Option<String>,
    pub charging_accessibility: Option<String>,
}

// ── Open-check endpoint (functionTypes=supercharger) ─────────────────────────

#[derive(Deserialize)]
pub struct OpenCheckResponse {
    pub data: OpenCheckData,
}

#[derive(Deserialize)]
pub struct OpenCheckData {
    pub supercharger_function: Option<OpenCheckSuperchargerFunction>,
    pub functions: Option<Vec<OpenCheckFunction>>,
}

#[derive(Deserialize)]
pub struct OpenCheckSuperchargerFunction {
    pub site_status: Option<String>,
    pub num_charger_stalls: Option<String>, // string in the API
    pub open_to_non_tesla: Option<bool>,
    pub installed_full_power: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenCheckFunction {
    pub opening_date: Option<String>, // "YYYY-MM-DD"
}

// ── Location-details endpoint ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LocationDetailsResponse {
    pub data: LocationDetailsData,
}

#[derive(Deserialize)]
pub struct LocationDetailsData {
    pub supercharger_function: Option<RawSuperchargerFunction>,
    #[serde(default)]
    pub functions: Option<Vec<RawFunction>>,
}

#[derive(Deserialize)]
pub struct RawSuperchargerFunction {
    pub customer_facing_coming_soon_date: Option<String>,
    pub coming_soon_name: Option<String>,
    pub project_status: Option<String>,
    pub num_charger_stalls: Option<String>,
    pub charging_accessibility: Option<String>,
}

#[derive(Deserialize)]
pub struct RawFunction {
    pub address: Option<RawAddress>,
}

#[derive(Deserialize)]
pub struct RawAddress {
    pub address_1: Option<String>,
    pub county: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// Merged detail payload consumed by the domain layer.
#[derive(Clone, Default)]
pub struct ComingSoonDetails {
    pub customer_facing_coming_soon_date: Option<String>,
    pub coming_soon_name: Option<String>,
    pub project_status: Option<String>,
    pub num_charger_stalls: Option<String>,
    pub charging_accessibility: Option<String>,
    pub street_address: Option<String>,
    pub county: Option<String>,
    pub postal_code: Option<String>,
    pub country_code: Option<String>,
}
