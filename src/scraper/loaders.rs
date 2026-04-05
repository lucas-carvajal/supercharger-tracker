use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::NaiveDate;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use serde::Deserialize;

use crate::domain::OpenResult;
use crate::scraper::raw::{ApiResponse, ComingSoonDetails, Location, LocationDetailsResponse, OpenCheckResponse};

const DETAILS_BATCH_SIZE: usize = 50;
const DETAILS_TIMEOUT_SECS: u64 = 10;

// ── Public result type ────────────────────────────────────────────────────────

pub struct LoadResult {
    pub locations: Vec<Location>,
    /// Details keyed by supercharger ID (the Tesla location URL slug).
    pub coming_soon_details: HashMap<String, ComingSoonDetails>,
    /// IDs where the details fetch failed outright (network error, timeout, block).
    /// Distinct from IDs that returned no `supercharger_function` — those are legitimate.
    pub failed_detail_ids: HashSet<String>,
}

// ── Browser-mode helper type ──────────────────────────────────────────────────

/// Wraps each browser-side fetch result so we can distinguish a genuine
/// network/parse failure (ok=false) from an API response with no details (ok=true, data=null).
#[derive(Deserialize)]
struct BrowserDetailResult {
    ok: bool,
    data: Option<LocationDetailsResponse>,
}

#[derive(Deserialize)]
struct BrowserOpenCheckResult {
    ok: bool,
    data: Option<OpenCheckResponse>,
}

// ── Public loaders ────────────────────────────────────────────────────────────

/// Fetch all coming-soon locations and their details using an already-authenticated
/// browser page. Does not launch or close Chrome — the caller owns the browser lifecycle.
pub async fn load_from_browser(
    country: &str,
    page: &Page,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
    println!("  → Fetching location data from inside the browser…");
    let json_text: String = page
        .evaluate(format!(
            "fetch('/api/findus/get-locations?country={country}').then(r => r.text())"
        ))
        .await?
        .into_value()?;

    if json_text.trim_start().starts_with('<') {
        eprintln!("  ✗ Got HTML — Akamai still blocking (try --show-browser to debug)");
        return Err("API returned HTML (access denied)".into());
    }

    let resp: ApiResponse = serde_json::from_str(&json_text)?;
    let locations = resp.data.data;
    let ids = coming_soon_ids(&locations);
    let total = ids.len();

    let num_batches = ids.chunks(DETAILS_BATCH_SIZE).count();
    println!(
        "  → Fetching details for {total} coming-soon/winner superchargers \
         ({num_batches} batches of {DETAILS_BATCH_SIZE}, {DETAILS_TIMEOUT_SECS}s timeout)…"
    );

    let (coming_soon_details, failed_detail_ids) =
        fetch_batch_details_from_page(page, ids).await;

    println!("  → Details done: {}/{total} resolved", coming_soon_details.len());

    Ok(LoadResult { locations, coming_soon_details, failed_detail_ids })
}

/// Check whether disappeared charger IDs have actually opened (gone live as superchargers).
///
/// Uses the `functionTypes=supercharger` endpoint. Returns `(confirmed_open, failed_ids)`:
/// - `confirmed_open`: map of id → OpenResult for chargers confirmed open
/// - `failed_ids`: IDs where the fetch itself failed (network error, timeout) — these
///   should be flagged for retry rather than marked REMOVED
///
/// IDs absent from both maps were checked successfully and are not open (presumed removed).
///
/// Takes an already-authenticated browser page — no additional Akamai wait needed.
pub async fn fetch_open_status_for_ids(
    page: &Page,
    ids: &[String],
) -> Result<(HashMap<String, OpenResult>, HashSet<String>), Box<dyn std::error::Error>> {
    let timeout_ms = DETAILS_TIMEOUT_SECS * 1000;
    let mut results: HashMap<String, OpenResult> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();

    let ids_vec: Vec<String> = ids.to_vec();
    let batch_json = serde_json::to_string(&ids_vec)?;

    let text: String = page
        .evaluate(format!(
            r#"
            (() => {{
                const slugs = {batch_json};
                return Promise.all(
                    slugs.map(slug =>
                        fetch(`/api/findus/get-location-details?locationSlug=${{slug}}&functionTypes=supercharger&locale=en_US&isInHkMoTw=false`,
                              {{ signal: AbortSignal.timeout({timeout_ms}) }})
                            .then(r => r.json())
                            .then(data => ({{ok: true, data}}))
                            .catch(() => ({{ok: false, data: null}}))
                    )
                ).then(results => JSON.stringify(slugs.map((s, i) => [s, results[i]])));
            }})()
            "#
        ))
        .await?
        .into_value()?;

    let pairs: Vec<(String, BrowserOpenCheckResult)> = serde_json::from_str(&text)?;

    for (id, result) in pairs {
        if !result.ok {
            eprintln!("  ⚠ Open-check fetch failed for {id} — flagging for retry");
            failed.insert(id);
            continue;
        }
        let Some(resp) = result.data else { continue };
        let Some(sf) = resp.data.supercharger_function else { continue };
        if sf.site_status.as_deref() != Some("open") {
            continue;
        }

        let opening_date = resp
            .data
            .functions
            .as_deref()
            .and_then(|fs| fs.first())
            .and_then(|f| f.opening_date.as_deref())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

        let num_stalls = sf
            .num_charger_stalls
            .as_deref()
            .and_then(|s| s.parse::<i32>().ok());

        results.insert(id, OpenResult {
            opening_date,
            num_stalls,
            open_to_non_tesla: sf.open_to_non_tesla,
        });
    }

    Ok((results, failed))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collect IDs (Tesla location URL slugs) for all coming-soon superchargers that have one.
fn coming_soon_ids(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .filter(|l| l.location_type.iter().any(|t| matches!(t.as_str(),
            "coming_soon_supercharger" | "winner_supercharger" | "current_winner_supercharger"
        )))
        .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
        .map(|l| l.location_url_slug.clone())
        .collect()
}

/// Launch Chrome (headless or visible), navigate to Tesla.com, and wait for Akamai cookies.
/// Returns the browser handle and the ready page — caller is responsible for closing the browser.
pub async fn launch_browser_and_wait(
    show_browser: bool,
) -> Result<(Browser, Page), Box<dyn std::error::Error>> {
    let chrome = find_chrome()?;

    println!(
        "Launching Chrome ({})…",
        if show_browser { "visible" } else { "headless" }
    );

    let stealth_args = [
        "--no-first-run",
        "--disable-extensions",
        "--disable-blink-features=AutomationControlled",
        "--excludeSwitches=enable-automation",
        "--window-size=1280,800",
    ];

    let config = if show_browser {
        let mut b = BrowserConfig::builder().chrome_executable(&chrome).with_head();
        for arg in &stealth_args {
            b = b.arg(*arg);
        }
        b.build()?
    } else {
        let mut b = BrowserConfig::builder().chrome_executable(&chrome);
        for arg in &stealth_args {
            b = b.arg(*arg);
        }
        b.build()?
    };

    let (browser, mut handler) = Browser::launch(config).await?;

    tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });

    // Open a blank page first — passing a URL to new_page() makes chromiumoxide
    // wait for the load event, which Akamai can block indefinitely.
    let page = browser.new_page("about:blank").await?;
    println!("  → Navigating to https://www.tesla.com/findus");
    let _ = page.evaluate("window.location.href = 'https://www.tesla.com/findus'").await;

    println!("  → Waiting for session cookies (Akamai)…");
    tokio::time::sleep(Duration::from_secs(8)).await;

    Ok((browser, page))
}

/// Fetch details for `ids` in batches from an already-authenticated browser page.
/// Returns `(details_map, failed_ids)`. Retries failed IDs once before returning.
pub async fn fetch_batch_details_from_page(
    page: &Page,
    ids: Vec<String>,
) -> (HashMap<String, ComingSoonDetails>, HashSet<String>) {
    let batches: Vec<&[String]> = ids.chunks(DETAILS_BATCH_SIZE).collect();
    let num_batches = batches.len();
    let timeout_ms = DETAILS_TIMEOUT_SECS * 1000;

    let mut details: HashMap<String, ComingSoonDetails> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();

    for (i, batch) in batches.iter().enumerate() {
        println!("  → Batch {}/{num_batches} ({} chargers)…", i + 1, batch.len());
        let batch_json = match serde_json::to_string(batch) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Some(pairs) = eval_detail_batch(page, &batch_json, timeout_ms).await {
            for (id, result) in pairs {
                if result.ok {
                    if let Some(d) = result.data.and_then(|r| r.data.supercharger_function) {
                        details.insert(id, d);
                    }
                } else {
                    failed.insert(id);
                }
            }
        } else {
            // Entire batch evaluation failed — mark all IDs in this batch as failed.
            failed.extend(batch.iter().cloned());
        }
    }

    // Retry any failed IDs once.
    if !failed.is_empty() {
        let retry_ids: Vec<String> = failed.iter().cloned().collect();
        failed.clear();
        eprintln!("  ⚠ {} detail fetches failed — retrying…", retry_ids.len());

        let batch_json = serde_json::to_string(&retry_ids).unwrap_or_default();
        match eval_detail_batch(page, &batch_json, timeout_ms).await {
            Some(pairs) => {
                for (id, result) in pairs {
                    if result.ok {
                        if let Some(d) = result.data.and_then(|r| r.data.supercharger_function) {
                            details.insert(id, d);
                        }
                    } else {
                        failed.insert(id);
                    }
                }
            }
            None => failed.extend(retry_ids),
        }

        if !failed.is_empty() {
            eprintln!("  ⚠ {} chargers still failed after retry", failed.len());
        }
    }

    (details, failed)
}

/// Run one detail-fetch batch inside the browser page.
/// Returns `None` if the JS evaluation or JSON parsing fails entirely.
async fn eval_detail_batch(
    page: &Page,
    batch_json: &str,
    timeout_ms: u64,
) -> Option<Vec<(String, BrowserDetailResult)>> {
    let text: String = page
        .evaluate(format!(
            r#"
            (() => {{
                const slugs = {batch_json};
                return Promise.all(
                    slugs.map(slug =>
                        fetch(`/api/findus/get-location-details?locationSlug=${{slug}}&functionTypes=coming_soon_supercharger&locale=en_US&isInHkMoTw=false`,
                              {{ signal: AbortSignal.timeout({timeout_ms}) }})
                            .then(r => r.json())
                            .then(data => ({{ok: true, data}}))
                            .catch(() => ({{ok: false, data: null}}))
                    )
                ).then(results => JSON.stringify(slugs.map((s, i) => [s, results[i]])));
            }})()
            "#
        ))
        .await
        .ok()?
        .into_value()
        .ok()?;

    serde_json::from_str(&text).ok()
}

fn find_chrome() -> Result<String, Box<dyn std::error::Error>> {
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }
    Err("Chrome not found — install Google Chrome from https://www.google.com/chrome/".into())
}
