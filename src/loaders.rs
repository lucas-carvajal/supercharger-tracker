use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::stream::{self, StreamExt};
use serde::Deserialize;

use crate::raw::{ApiResponse, ComingSoonDetails, Location, LocationDetailsResponse};

const API_URL: &str = "https://www.tesla.com/api/findus/get-locations";
const DETAILS_URL: &str = "https://www.tesla.com/api/findus/get-location-details";
const DETAILS_CONCURRENCY: usize = 10;
const DETAILS_BATCH_SIZE: usize = 50;
const DETAILS_TIMEOUT_SECS: u64 = 10;
const DETAILS_RETRY_TIMEOUT_SECS: u64 = 20;

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

// ── Public loaders ────────────────────────────────────────────────────────────

pub async fn load_from_file(path: &str) -> Result<LoadResult, Box<dyn std::error::Error>> {
    println!("Reading from file: {path}");
    let raw = fs::read_to_string(path)?;
    let resp: ApiResponse = serde_json::from_str(&raw)?;
    Ok(LoadResult {
        locations: resp.data.data,
        coming_soon_details: HashMap::new(),
        failed_detail_ids: HashSet::new(),
    })
}

pub async fn load_with_cookie(
    country: &str,
    cookie: &str,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
    let url = format!("{API_URL}?country={country}");
    println!("Fetching via HTTP: {url}");

    let client = build_cookie_client(cookie, DETAILS_TIMEOUT_SECS)?;

    let response = client.get(&url).send().await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() || !body.trim_start().starts_with('{') {
        eprintln!("  ✗ HTTP {status}");
        eprintln!("  ✗ Response: {}", &body[..body.len().min(300)]);
        return Err(format!("API returned non-JSON response (HTTP {status})").into());
    }

    let resp: ApiResponse = serde_json::from_str(&body)?;
    let locations = resp.data.data;
    let ids = coming_soon_ids(&locations);

    let (coming_soon_details, failed_detail_ids) =
        fetch_details_only_cookie(cookie, ids).await?;

    Ok(LoadResult { locations, coming_soon_details, failed_detail_ids })
}

pub async fn load_from_browser(
    country: &str,
    show_browser: bool,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
    let (mut browser, page) = launch_browser_and_wait(show_browser).await?;

    println!("  → Fetching location data from inside the browser…");
    let json_text: String = page
        .evaluate(format!(
            "fetch('/api/findus/get-locations?country={country}').then(r => r.text())"
        ))
        .await?
        .into_value()?;

    if json_text.trim_start().starts_with('<') {
        eprintln!("  ✗ Got HTML — Akamai still blocking (try --show-browser to debug)");
        browser.close().await.ok();
        return Err("API returned HTML (access denied)".into());
    }

    let resp: ApiResponse = serde_json::from_str(&json_text)?;
    let locations = resp.data.data;
    let ids = coming_soon_ids(&locations);
    let total = ids.len();

    let num_batches = ids.chunks(DETAILS_BATCH_SIZE).count();
    println!(
        "  → Fetching details for {total} coming-soon superchargers \
         ({num_batches} batches of {DETAILS_BATCH_SIZE}, {DETAILS_TIMEOUT_SECS}s timeout)…"
    );

    let (coming_soon_details, failed_detail_ids) =
        fetch_batch_details_from_page(&page, ids).await;

    println!("  → Details done: {}/{total} resolved", coming_soon_details.len());
    browser.close().await.ok();

    Ok(LoadResult { locations, coming_soon_details, failed_detail_ids })
}

// ── Details-only loaders (used by retry-failed command) ──────────────────────

/// Fetch details for a specific set of charger IDs using a cookie-authenticated HTTP client.
/// Includes one automatic retry with a longer timeout for any failed requests.
pub async fn fetch_details_only_cookie(
    cookie: &str,
    ids: Vec<String>,
) -> Result<(HashMap<String, ComingSoonDetails>, HashSet<String>), Box<dyn std::error::Error>> {
    let total = ids.len();
    println!(
        "  → Fetching details for {total} chargers \
         ({DETAILS_CONCURRENCY} concurrent, {DETAILS_TIMEOUT_SECS}s timeout)…"
    );
    let client = build_cookie_client(cookie, DETAILS_TIMEOUT_SECS)?;
    let (mut details, mut failed) = fetch_details_with_client(&client, ids).await;

    if !failed.is_empty() {
        let retry_count = failed.len();
        eprintln!(
            "  ⚠ {retry_count} detail fetches failed — retrying with {DETAILS_RETRY_TIMEOUT_SECS}s timeout…"
        );
        let retry_client = build_cookie_client(cookie, DETAILS_RETRY_TIMEOUT_SECS)?;
        let retry_ids: Vec<String> = failed.into_iter().collect();
        let (retry_details, still_failed) =
            fetch_details_with_client(&retry_client, retry_ids).await;
        details.extend(retry_details);
        failed = still_failed;
        if !failed.is_empty() {
            eprintln!("  ⚠ {} chargers still failed after retry", failed.len());
        }
    }

    println!("  → Details done: {}/{total} resolved", details.len());
    Ok((details, failed))
}

/// Fetch details for a specific set of charger IDs using a browser session for Akamai auth.
/// Launches Chrome, waits for Akamai cookies, then fetches only the requested IDs.
pub async fn fetch_details_only_browser(
    ids: Vec<String>,
    show_browser: bool,
) -> Result<(HashMap<String, ComingSoonDetails>, HashSet<String>), Box<dyn std::error::Error>> {
    let total = ids.len();
    let num_batches = ids.chunks(DETAILS_BATCH_SIZE).count();
    println!(
        "  → Fetching details for {total} chargers \
         ({num_batches} batches of {DETAILS_BATCH_SIZE}, {DETAILS_TIMEOUT_SECS}s timeout)…"
    );

    let (mut browser, page) = launch_browser_and_wait(show_browser).await?;
    let (details, failed) = fetch_batch_details_from_page(&page, ids).await;

    println!("  → Details done: {}/{total} resolved", details.len());
    browser.close().await.ok();

    Ok((details, failed))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collect IDs (Tesla location URL slugs) for all coming-soon superchargers that have one.
fn coming_soon_ids(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .filter(|l| l.location_type.iter().any(|t| t == "coming_soon_supercharger"))
        .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
        .map(|l| l.location_url_slug.clone())
        .collect()
}

/// Build a reqwest client with Tesla cookie headers and the given timeout.
fn build_cookie_client(
    cookie: &str,
    timeout_secs: u64,
) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
            .parse()?,
    );
    headers.insert(reqwest::header::REFERER, "https://www.tesla.com/findus".parse()?);
    headers.insert(reqwest::header::ACCEPT, "application/json".parse()?);
    headers.insert(reqwest::header::COOKIE, cookie.parse()?);
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(timeout_secs))
        .build()?)
}

/// Launch Chrome (headless or visible), navigate to Tesla.com, and wait for Akamai cookies.
/// Returns the browser handle and the ready page — caller is responsible for closing the browser.
async fn launch_browser_and_wait(
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
async fn fetch_batch_details_from_page(
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

/// Fetch location details for a list of charger IDs concurrently using reqwest.
///
/// IDs are passed to the Tesla API as `locationSlug` URL parameters — this is the
/// Tesla API's name for what we call our system ID.
///
/// Returns `(details_map, failed_ids)` where `failed_ids` contains IDs whose HTTP
/// request failed outright (network error, timeout, non-JSON response). IDs that
/// returned a successful response but had no `supercharger_function` are silently
/// omitted from both maps — that is a legitimate API state.
async fn fetch_details_with_client(
    client: &reqwest::Client,
    ids: Vec<String>,
) -> (HashMap<String, ComingSoonDetails>, HashSet<String>) {
    let total = ids.len();
    let done = Arc::new(AtomicUsize::new(0));

    // (id, request_succeeded, details_opt)
    let outcomes: Vec<(String, bool, Option<ComingSoonDetails>)> = stream::iter(ids)
        .map(|id| {
            let client = client.clone();
            let done = done.clone();
            async move {
                let url = format!(
                    "{DETAILS_URL}?locationSlug={id}&functionTypes=coming_soon_supercharger&locale=en_US&isInHkMoTw=false"
                );
                let result: Result<Option<ComingSoonDetails>, reqwest::Error> = async {
                    let resp = client.get(&url).send().await?;
                    let response: LocationDetailsResponse = resp.json().await?;
                    Ok(response.data.supercharger_function)
                }
                .await;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 10 == 0 || n == total {
                    println!("  → Details: {n}/{total}");
                }
                match result {
                    Ok(details_opt) => (id, true, details_opt),
                    Err(_) => (id, false, None),
                }
            }
        })
        .buffer_unordered(DETAILS_CONCURRENCY)
        .collect()
        .await;

    let mut details_map = HashMap::new();
    let mut failed = HashSet::new();
    for (id, ok, details_opt) in outcomes {
        if ok {
            if let Some(d) = details_opt {
                details_map.insert(id, d);
            }
        } else {
            failed.insert(id);
        }
    }
    (details_map, failed)
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
