use std::sync::LazyLock;

use country_boundaries::{BOUNDARIES_ODBL_360X180, CountryBoundaries, LatLon};

static BOUNDARIES: LazyLock<CountryBoundaries> = LazyLock::new(|| {
    CountryBoundaries::from_reader(BOUNDARIES_ODBL_360X180)
        .expect("embedded country boundary dataset must parse")
});

/// ISO 3166-1 alpha-2 country for a point, or `None` if the coords are invalid or not on land.
///
/// `ids()` is smallest-area first (`US-TX`, then `US`). Keep 2-letter codes with no hyphen
/// and take the last one so overlapping claims resolve to the largest area.
pub fn country_from_coords(lat: f64, lng: f64) -> Option<String> {
    let Ok(point) = LatLon::new(lat, lng) else {
        tracing::warn!(lat, lng, "invalid coordinates for country lookup");
        return None;
    };

    let iso2 = BOUNDARIES
        .ids(point)
        .into_iter()
        .rfind(|id| id.len() == 2 && !id.contains('-'))
        .map(str::to_owned);

    if iso2.is_none() {
        tracing::warn!(lat, lng, "no land country match for coordinates");
    }

    iso2
}

#[cfg(test)]
mod tests {
    use super::country_from_coords;

    #[test]
    fn austin_is_us() {
        assert_eq!(
            country_from_coords(30.2672, -97.7431).as_deref(),
            Some("US")
        );
    }

    #[test]
    fn highbridge_is_gb() {
        assert_eq!(
            country_from_coords(51.22962, -2.959685).as_deref(),
            Some("GB")
        );
    }

    #[test]
    fn mid_ocean_is_none() {
        assert_eq!(country_from_coords(0.0, 0.0), None);
    }
}
