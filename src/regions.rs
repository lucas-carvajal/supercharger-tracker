/// Resolve a `?region=` query parameter value to the list of DB `region` strings
/// that should be matched. Returns `None` for unknown/invalid inputs.
///
/// Input is matched case-insensitively. The returned strings are exact DB spellings.
pub fn resolve(input: &str) -> Option<Vec<String>> {
    let lower = input.to_lowercase();
    let result = match lower.as_str() {
        // ── Aggregate: United States ──────────────────────────────────────────
        "us" => vec![
            "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN",
            "IA", "KS", "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV",
            "NH", "NJ", "NM", "NY", "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN",
            "TX", "UT", "VT", "VA", "WA", "WV", "WI", "WY", "DC",
        ]
        .into_iter()
        .map(String::from)
        .collect(),

        // ── Aggregate: Australia ──────────────────────────────────────────────
        "au" | "australia" => vec!["NSW", "VIC", "QLD", "SA", "WA", "TAS", "NT", "ACT"]
            .into_iter()
            .map(String::from)
            .collect(),

        // ── Aggregate: Canada ─────────────────────────────────────────────────
        "canada" => vec![
            "BC", "ON", "AB", "SK", "MB", "QC", "NB", "NS", "PE", "NL", "NT", "YT", "NU",
        ]
        .into_iter()
        .map(String::from)
        .collect(),

        // ── Aggregate: Mexico ─────────────────────────────────────────────────
        "mexico" => vec![
            "Mexico", "AGU", "BCN", "BCS", "CAM", "CHP", "CHH", "COA", "COAH", "COL", "CMX",
            "DUR", "GUA", "GRO", "HID", "JAL", "MEX", "MIC", "MOR", "NAY", "NLE", "OAX", "PUE",
            "QUE", "ROO", "SLP", "SIN", "SON", "TAB", "TAM", "TLX", "VER", "YUC", "ZAC",
        ]
        .into_iter()
        .map(String::from)
        .collect(),

        // ── Multi-variant countries ───────────────────────────────────────────
        "united kingdom" | "uk" => vec!["United Kingdom", "UK"]
            .into_iter()
            .map(String::from)
            .collect(),

        "turkey" | "turkiye" | "türkiye" => vec!["Türkiye", "Turkiye"]
            .into_iter()
            .map(String::from)
            .collect(),

        "uae" | "united arab emirates" => {
            vec!["United Arab Emirates", "UAE", "UAE - Dubai Silicon Oasis"]
                .into_iter()
                .map(String::from)
                .collect()
        }

        "new zealand" | "nz" => vec!["New Zealand", "NZ"]
            .into_iter()
            .map(String::from)
            .collect(),

        // ── Individual US state codes ─────────────────────────────────────────
        "al" => vec!["AL".to_string()],
        "ak" => vec!["AK".to_string()],
        "az" => vec!["AZ".to_string()],
        "ar" => vec!["AR".to_string()],
        "ca" => vec!["CA".to_string()],
        "co" => vec!["CO".to_string()],
        "ct" => vec!["CT".to_string()],
        "de" => vec!["DE".to_string()],
        "fl" => vec!["FL".to_string()],
        "ga" => vec!["GA".to_string()],
        "hi" => vec!["HI".to_string()],
        "id" => vec!["ID".to_string()],
        "il" => vec!["IL".to_string()],
        "in" => vec!["IN".to_string()],
        "ia" => vec!["IA".to_string()],
        "ks" => vec!["KS".to_string()],
        "ky" => vec!["KY".to_string()],
        "la" => vec!["LA".to_string()],
        "me" => vec!["ME".to_string()],
        "md" => vec!["MD".to_string()],
        "ma" => vec!["MA".to_string()],
        "mi" => vec!["MI".to_string()],
        "mn" => vec!["MN".to_string()],
        "ms" => vec!["MS".to_string()],
        "mo" => vec!["MO".to_string()],
        "mt" => vec!["MT".to_string()],
        "ne" => vec!["NE".to_string()],
        "nv" => vec!["NV".to_string()],
        "nh" => vec!["NH".to_string()],
        "nj" => vec!["NJ".to_string()],
        "nm" => vec!["NM".to_string()],
        "ny" => vec!["NY".to_string()],
        "nc" => vec!["NC".to_string()],
        "nd" => vec!["ND".to_string()],
        "oh" => vec!["OH".to_string()],
        "ok" => vec!["OK".to_string()],
        "or" => vec!["OR".to_string()],
        "pa" => vec!["PA".to_string()],
        "ri" => vec!["RI".to_string()],
        "sc" => vec!["SC".to_string()],
        "sd" => vec!["SD".to_string()],
        "tn" => vec!["TN".to_string()],
        "tx" => vec!["TX".to_string()],
        "ut" => vec!["UT".to_string()],
        "vt" => vec!["VT".to_string()],
        "va" => vec!["VA".to_string()],
        "wa" => vec!["WA".to_string()],
        "wv" => vec!["WV".to_string()],
        "wi" => vec!["WI".to_string()],
        "wy" => vec!["WY".to_string()],
        "dc" => vec!["DC".to_string()],

        // ── Individual AU territory codes ─────────────────────────────────────
        "nsw" => vec!["NSW".to_string()],
        "vic" => vec!["VIC".to_string()],
        "qld" => vec!["QLD".to_string()],
        "sa" => vec!["SA".to_string()],
        // "wa" already covered by US state above — same DB value
        "tas" => vec!["TAS".to_string()],
        "nt" => vec!["NT".to_string()],
        "act" => vec!["ACT".to_string()],

        // ── Individual Canadian province codes ────────────────────────────────
        "bc" => vec!["BC".to_string()],
        "on" => vec!["ON".to_string()],
        "ab" => vec!["AB".to_string()],
        "sk" => vec!["SK".to_string()],
        "mb" => vec!["MB".to_string()],
        "qc" => vec!["QC".to_string()],
        "nb" => vec!["NB".to_string()],
        "ns" => vec!["NS".to_string()],
        "pe" => vec!["PE".to_string()],
        "nl" => vec!["NL".to_string()],
        "yt" => vec!["YT".to_string()],
        "nu" => vec!["NU".to_string()],

        // ── Individual Mexican state codes ────────────────────────────────────
        "agu" => vec!["AGU".to_string()],
        "bcn" => vec!["BCN".to_string()],
        "bcs" => vec!["BCS".to_string()],
        "cam" => vec!["CAM".to_string()],
        "chp" => vec!["CHP".to_string()],
        "chh" => vec!["CHH".to_string()],
        "coa" => vec!["COA".to_string()],
        "coah" => vec!["COAH".to_string()],
        "col" => vec!["COL".to_string()],
        "cmx" => vec!["CMX".to_string()],
        "dur" => vec!["DUR".to_string()],
        "gua" => vec!["GUA".to_string()],
        "gro" => vec!["GRO".to_string()],
        "hid" => vec!["HID".to_string()],
        "jal" => vec!["JAL".to_string()],
        "mex" => vec!["MEX".to_string()],
        "mic" => vec!["MIC".to_string()],
        "mor" => vec!["MOR".to_string()],
        "nay" => vec!["NAY".to_string()],
        "nle" => vec!["NLE".to_string()],
        "oax" => vec!["OAX".to_string()],
        "pue" => vec!["PUE".to_string()],
        "que" => vec!["QUE".to_string()],
        "roo" => vec!["ROO".to_string()],
        "slp" => vec!["SLP".to_string()],
        "sin" => vec!["SIN".to_string()],
        "son" => vec!["SON".to_string()],
        "tab" => vec!["TAB".to_string()],
        "tam" => vec!["TAM".to_string()],
        "tlx" => vec!["TLX".to_string()],
        "ver" => vec!["VER".to_string()],
        "yuc" => vec!["YUC".to_string()],
        "zac" => vec!["ZAC".to_string()],

        // ── Single-variant countries ──────────────────────────────────────────
        "germany" => vec!["Germany".to_string()],
        "france" => vec!["France".to_string()],
        "spain" => vec!["Spain".to_string()],
        "norway" => vec!["Norway".to_string()],
        "sweden" => vec!["Sweden".to_string()],
        "italy" => vec!["Italy".to_string()],
        "finland" => vec!["Finland".to_string()],
        "denmark" => vec!["Denmark".to_string()],
        "hungary" => vec!["Hungary".to_string()],
        "romania" => vec!["Romania".to_string()],
        "czech republic" => vec!["Czech Republic".to_string()],
        "iceland" => vec!["Iceland".to_string()],
        "ireland" => vec!["Ireland".to_string()],
        "portugal" => vec!["Portugal".to_string()],
        "croatia" => vec!["Croatia".to_string()],
        "slovenia" => vec!["Slovenia".to_string()],
        "slovakia" => vec!["Slovakia".to_string()],
        "switzerland" => vec!["Switzerland".to_string()],
        "austria" => vec!["Austria".to_string()],
        "netherlands" => vec!["Netherlands".to_string()],
        "poland" => vec!["Poland".to_string()],
        "latvia" => vec!["Latvia".to_string()],
        "morocco" => vec!["Morocco".to_string()],
        "taiwan" => vec!["Taiwan".to_string()],
        "thailand" => vec!["Thailand".to_string()],
        "japan" => vec!["Japan".to_string()],
        "south korea" => vec!["South Korea".to_string()],
        "chile" => vec!["Chile".to_string()],
        "colombia" => vec!["Colombia".to_string()],
        "israel" => vec!["Israel".to_string()],
        "saudi arabia" => vec!["Saudi Arabia".to_string()],

        // ── Unknown ───────────────────────────────────────────────────────────
        _ => {
            eprintln!("[region-filter] unknown region requested: {input:?}");
            return None;
        }
    };
    Some(result)
}

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
