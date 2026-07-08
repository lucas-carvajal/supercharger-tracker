use serde::Deserialize;

#[derive(Deserialize)]
pub struct ApiResponse {
    pub data: DataWrapper,
}

#[derive(Deserialize)]
pub struct DataWrapper {
    pub data: Vec<Location>,
}

#[derive(Deserialize)]
pub struct Location {
    #[allow(dead_code)]
    pub uuid: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub location_type: Vec<String>,
    pub location_url_slug: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub supercharger_function: Option<SuperchargerFunction>,
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
