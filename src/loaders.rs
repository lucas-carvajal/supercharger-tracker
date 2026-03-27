use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chromiumoxide::{Browser, BrowserConfig};
use futures::stream::{self, StreamExt};

use crate::raw::{ApiResponse, ComingSoonDetails, Location, LocationDetailsResponse};

const API_URL: &str = "https://www.tesla.com/api/findus/get-locations";
const DETAILS_URL: &str = "https://www.tesla.com/api/findus/get-location-details";
const DETAILS_CONCURRENCY: usize = 10;
const DETAILS_BATCH_SIZE: usize = 50;
const DETAILS_TIMEOUT_SECS: u64 = 10;

// ── Public result type ────────────────────────────────────────────────────────

pub struct LoadResult {
    pub locations: Vec<Location>,
    /// Details keyed by `location_url_slug` (the numeric slug string).
    pub coming_soon_details: HashMap<String, ComingSoonDetails>,
}

// ── Public loaders ────────────────────────────────────────────────────────────

pub async fn load_from_file(path: &str) -> Result<LoadResult, Box<dyn std::error::Error>> {
    println!("Reading from file: {path}");
    let raw = fs::read_to_string(path)?;
    let resp: ApiResponse = serde_json::from_str(&raw)?;
    Ok(LoadResult {
        locations: resp.data.data,
        coming_soon_details: HashMap::new(),
    })
}

pub async fn load_with_cookie(
    country: &str,
    cookie: &str,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
    let url = format!("{API_URL}?country={country}");
    println!("Fetching via HTTP: {url}");

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

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(DETAILS_TIMEOUT_SECS))
        .build()?;

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

    let slugs = coming_soon_slugs(&locations);
    let total = slugs.len();
    println!("  → Fetching details for {total} coming-soon superchargers ({DETAILS_CONCURRENCY} concurrent, {DETAILS_TIMEOUT_SECS}s timeout)…");
    let coming_soon_details = fetch_details_with_client(&client, slugs).await;
    println!("  → Details done: {}/{total} resolved", coming_soon_details.len());

    Ok(LoadResult { locations, coming_soon_details })
}

pub async fn load_from_browser(
    country: &str,
    show_browser: bool,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
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

    let (mut browser, mut handler) = Browser::launch(config).await?;

    tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });

    // Open a blank page first — passing a URL to new_page() makes chromiumoxide
    // wait for the load event, which Akamai can block indefinitely.
    let page = browser.new_page("about:blank").await?;
    println!("  → Navigating to https://www.tesla.com/findus");
    // Trigger navigation via JS: evaluate() returns immediately and the
    // navigation happens in the background while we sleep below.
    let _ = page.evaluate("window.location.href = 'https://www.tesla.com/findus'").await;

    println!("  → Waiting for session cookies (Akamai)…");
    tokio::time::sleep(Duration::from_secs(8)).await;

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

    // Fetch coming-soon details in batches from inside the browser.
    let slugs = coming_soon_slugs(&locations);
    let total = slugs.len();
    let batches: Vec<&[String]> = slugs.chunks(DETAILS_BATCH_SIZE).collect();
    let num_batches = batches.len();
    println!("  → Fetching details for {total} coming-soon superchargers ({num_batches} batches of {DETAILS_BATCH_SIZE}, {DETAILS_TIMEOUT_SECS}s timeout)…");

    let mut coming_soon_details: HashMap<String, ComingSoonDetails> = HashMap::new();
    let timeout_ms = DETAILS_TIMEOUT_SECS * 1000;

    for (i, batch) in batches.iter().enumerate() {
        println!("  → Batch {}/{num_batches} ({} slugs)…", i + 1, batch.len());
        let batch_json = serde_json::to_string(batch)?;
        let details_text: String = page
            .evaluate(format!(
                r#"
                (() => {{
                    const slugs = {batch_json};
                    return Promise.all(
                        slugs.map(slug =>
                            fetch(`/api/findus/get-location-details?locationSlug=${{slug}}&functionTypes=coming_soon_supercharger&locale=en_US&isInHkMoTw=false`,
                                  {{ signal: AbortSignal.timeout({timeout_ms}) }})
                                .then(r => r.json())
                                .catch(() => null)
                        )
                    ).then(results => JSON.stringify(slugs.map((s, i) => [s, results[i]])));
                }})()
                "#
            ))
            .await?
            .into_value()?;

        let raw_pairs: Vec<(String, Option<LocationDetailsResponse>)> =
            serde_json::from_str(&details_text)?;
        coming_soon_details.extend(
            raw_pairs
                .into_iter()
                .filter_map(|(slug, resp)| resp?.data.supercharger_function.map(|d| (slug, d))),
        );
    }

    println!("  → Details done: {}/{total} resolved", coming_soon_details.len());
    browser.close().await.ok();

    Ok(LoadResult { locations, coming_soon_details })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collect non-null location slugs for all coming-soon superchargers.
fn coming_soon_slugs(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .filter(|l| l.location_type.iter().any(|t| t == "coming_soon_supercharger"))
        .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
        .map(|l| l.location_url_slug.clone())
        .collect()
}

/// Fetch location details for a list of slugs concurrently using reqwest,
/// logging progress every 10 completions.
async fn fetch_details_with_client(
    client: &reqwest::Client,
    slugs: Vec<String>,
) -> HashMap<String, ComingSoonDetails> {
    let total = slugs.len();
    let done = Arc::new(AtomicUsize::new(0));

    stream::iter(slugs)
        .map(|slug| {
            let client = client.clone();
            let done = done.clone();
            async move {
                let url = format!(
                    "{DETAILS_URL}?locationSlug={slug}&functionTypes=coming_soon_supercharger&locale=en_US&isInHkMoTw=false"
                );
                let result = async {
                    let resp = client.get(&url).send().await.ok()?;
                    let details: LocationDetailsResponse = resp.json().await.ok()?;
                    details.data.supercharger_function.map(|d| (slug, d))
                }
                .await;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 10 == 0 || n == total {
                    println!("  → Details: {n}/{total}");
                }
                result
            }
        })
        .buffer_unordered(DETAILS_CONCURRENCY)
        .filter_map(|r| async move { r })
        .collect()
        .await
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
