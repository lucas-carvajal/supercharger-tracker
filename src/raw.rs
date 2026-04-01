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
    pub uuid: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub location_type: Vec<String>,
    pub location_url_slug: String,
    #[serde(default)]
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

// ── Location-details endpoint ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LocationDetailsResponse {
    pub data: LocationDetailsData,
}

#[derive(Deserialize)]
pub struct LocationDetailsData {
    pub supercharger_function: Option<ComingSoonDetails>,
}

#[derive(Deserialize, Clone)]
pub struct ComingSoonDetails {
    pub customer_facing_coming_soon_date: Option<String>,
    pub coming_soon_name: Option<String>,
}
