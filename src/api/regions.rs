// ── Static data ───────────────────────────────────────────────────────────────

const US_STATES: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN", "IA",
    "KS", "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ",
    "NM", "NY", "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT",
    "VA", "WA", "WV", "WI", "WY", "DC",
];

const AU_TERRITORIES: &[&str] = &["NSW", "VIC", "QLD", "SA", "WA", "TAS", "NT", "ACT"];

// NT (Northwest Territories) overlaps with AU; both map to the same DB value.
const CA_PROVINCES: &[&str] = &[
    "BC", "ON", "AB", "SK", "MB", "QC", "NB", "NS", "PE", "NL", "NT", "YT", "NU",
];

const MX_STATES: &[&str] = &[
    "AGU", "BCN", "BCS", "CAM", "CHP", "CHH", "COA", "COAH", "COL", "CMX", "DUR", "GUA", "GRO",
    "HID", "JAL", "MEX", "MIC", "MOR", "NAY", "NLE", "OAX", "PUE", "QUE", "ROO", "SLP", "SIN",
    "SON", "TAB", "TAM", "TLX", "VER", "YUC", "ZAC",
];

/// Countries with a single DB spelling. Case-insensitive match on input;
/// the string here is the exact canonical DB value.
const COUNTRIES: &[&str] = &[
    "Germany",
    "France",
    "Spain",
    "Norway",
    "Sweden",
    "Italy",
    "Finland",
    "Denmark",
    "Hungary",
    "Romania",
    "Czech Republic",
    "Iceland",
    "Ireland",
    "Portugal",
    "Croatia",
    "Slovenia",
    "Slovakia",
    "Switzerland",
    "Austria",
    "Netherlands",
    "Poland",
    "Latvia",
    "Morocco",
    "Taiwan",
    "Thailand",
    "Japan",
    "South Korea",
    "Chile",
    "Colombia",
    "Israel",
    "Saudi Arabia",
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Resolve a `?region=` query parameter value to the list of DB `region` strings
/// that should be matched. Returns `None` for unknown/invalid inputs.
///
/// Input is matched case-insensitively. The returned strings are exact DB spellings.
pub fn resolve(input: &str) -> Option<Vec<String>> {
    let lower = input.to_lowercase();

    // ── Aggregates and multi-variant countries ────────────────────────────────
    match lower.as_str() {
        "us" => return Some(strs(US_STATES)),
        "au" | "australia" => return Some(strs(AU_TERRITORIES)),
        "canada" => return Some(strs(CA_PROVINCES)),
        "mexico" => {
            let mut v = vec!["Mexico".to_string()];
            v.extend(strs(MX_STATES));
            return Some(v);
        }
        "united kingdom" | "uk" => return Some(vec!["United Kingdom".into(), "UK".into()]),
        "turkey" | "turkiye" | "türkiye" => {
            return Some(vec!["Türkiye".into(), "Turkiye".into()])
        }
        "uae" | "united arab emirates" => {
            return Some(vec![
                "United Arab Emirates".into(),
                "UAE".into(),
                "UAE - Dubai Silicon Oasis".into(),
            ])
        }
        "new zealand" | "nz" => return Some(vec!["New Zealand".into(), "NZ".into()]),
        _ => {}
    }

    // ── Individual sub-national codes (pass-through after validation) ─────────
    let upper = input.to_uppercase();
    let is_known_code = [US_STATES, AU_TERRITORIES, CA_PROVINCES, MX_STATES]
        .iter()
        .any(|group| group.contains(&upper.as_str()));
    if is_known_code {
        return Some(vec![upper]);
    }

    // ── Single-variant countries ──────────────────────────────────────────────
    if let Some(&canonical) = COUNTRIES.iter().find(|&&c| c.to_lowercase() == lower) {
        return Some(vec![canonical.into()]);
    }

    eprintln!("[region-filter] unknown region requested: {input:?}");
    None
}

fn strs(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|&s| s.into()).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_aggregate_returns_all_states() {
        let r = resolve("US").unwrap();
        assert!(r.contains(&"CA".to_string()));
        assert!(r.contains(&"DC".to_string()));
        assert_eq!(r.len(), 51); // 50 states + DC
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(resolve("denmark"), resolve("Denmark"));
        assert_eq!(resolve("germany"), resolve("GERMANY"));
    }

    #[test]
    fn uk_multi_variant() {
        let r = resolve("uk").unwrap();
        assert!(r.contains(&"United Kingdom".to_string()));
        assert!(r.contains(&"UK".to_string()));
    }

    #[test]
    fn nz_multi_variant() {
        let r = resolve("New Zealand").unwrap();
        assert!(r.contains(&"New Zealand".to_string()));
        assert!(r.contains(&"NZ".to_string()));
    }

    #[test]
    fn unknown_returns_none() {
        assert!(resolve("Bogusland").is_none());
        assert!(resolve("").is_none());
    }

    #[test]
    fn ca_is_california_not_canada() {
        let r = resolve("CA").unwrap();
        assert_eq!(r, vec!["CA"]);
    }

    #[test]
    fn canada_aggregate() {
        let r = resolve("Canada").unwrap();
        assert!(r.contains(&"ON".to_string()));
        assert!(r.contains(&"BC".to_string()));
        assert!(!r.contains(&"CA".to_string()));
    }

    #[test]
    fn mexico_aggregate_includes_full_name_variant() {
        let r = resolve("Mexico").unwrap();
        assert!(r.contains(&"Mexico".to_string()));
        assert!(r.contains(&"BCS".to_string()));
    }

    #[test]
    fn individual_mexican_state() {
        let r = resolve("BCS").unwrap();
        assert_eq!(r, vec!["BCS"]);
    }
}
