use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use chromiumoxide::{Browser, BrowserConfig, Page};
use chrono::NaiveDate;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use tempfile::TempDir;

use crate::domain::OpenResult;
use crate::scraper::raw::{
    ApiResponse, ComingSoonDetails, Location, LocationDetailsResponse, OpenCheckResponse,
};

pub const DETAILS_BATCH_SIZE: usize = 5;
const OPEN_STATUS_BATCH_SIZE: usize = 5;
const DETAILS_TIMEOUT_SECS: u64 = 10;
pub const DETAILS_BATCH_DELAY_MS: u64 = 1_200;
const DETAILS_FAILURE_BACKOFF_MS: u64 = 2_500;
const AKAMAI_INITIAL_SETTLE_SECS: u64 = 8;
const HEADLESS_AKAMAI_TIMEOUT_SECS: u64 = 30;
const VISIBLE_AKAMAI_TIMEOUT_SECS: u64 = 180;

// ── Public result type ────────────────────────────────────────────────────────

pub struct LoadResult {
    pub locations: Vec<Location>,
    /// Details keyed by supercharger ID (the Tesla location URL slug).
    pub coming_soon_details: HashMap<String, ComingSoonDetails>,
    /// IDs where the details fetch failed outright (network error, timeout, block).
    /// Distinct from IDs that returned no `supercharger_function` — those are legitimate.
    pub failed_detail_ids: HashSet<String>,
    pub unknown_enum_tracker: UnknownEnumTracker,
}

pub struct BrowserSession {
    pub browser: Browser,
    pub page: Page,
    _profile_dir: TempDir,
}

#[derive(Default)]
pub struct DetailBatchFetchResult {
    pub details: HashMap<String, ComingSoonDetails>,
    pub failed_ids: HashSet<String>,
    pub resolved_without_details: usize,
    pub failure_reasons: HashMap<String, usize>,
    pub blocked: bool,
}

impl DetailBatchFetchResult {
    fn resolved_count(&self, attempted: usize) -> usize {
        attempted.saturating_sub(self.failed_ids.len())
    }
}

pub const KNOWN_PROJECT_STATUS: &[&str] = &["Preliminary", "Design", "Construction", "Open"];
pub const KNOWN_CHARGING_ACCESSIBILITY: &[&str] = &[
    "Tesla Only",
    "All Vehicles (Production)",
    "NACS Partner Enabled (Production)",
];

/// Deduped warn-on-unknown for enum-like Tesla fields seen during a scrape/retry run.
#[derive(Default)]
pub struct UnknownEnumTracker {
    seen: HashMap<(&'static str, String), usize>,
}

impl UnknownEnumTracker {
    pub fn record(&mut self, field: &'static str, value: &str, known: &[&str]) {
        if known.contains(&value) {
            return;
        }
        let entry = self.seen.entry((field, value.to_string())).or_insert(0);
        if *entry == 0 {
            tracing::warn!(
                field,
                value,
                "unrecognised enum value — first seen this run"
            );
        }
        *entry += 1;
    }

    pub fn log_summary(&self) {
        if self.seen.is_empty() {
            return;
        }
        for ((field, value), count) in &self.seen {
            tracing::warn!(field, value, count, "unrecognised enum value (run total)");
        }
        tracing::warn!(
            distinct = self.seen.len(),
            "run saw unrecognised enum values — review and extend the KNOWN_* sets"
        );
    }

    #[cfg(test)]
    pub fn count(&self, field: &'static str, value: &str) -> usize {
        *self.seen.get(&(field, value.to_string())).unwrap_or(&0)
    }
}

impl BrowserSession {
    pub async fn close(mut self) {
        self.browser.close().await.ok();
    }
}

// ── Browser-mode helper type ──────────────────────────────────────────────────

/// Wraps each browser-side fetch result so we can distinguish a genuine
/// network/parse failure (ok=false) from an API response with no details (ok=true, data=null).
#[derive(Deserialize)]
struct BrowserDetailResult {
    ok: bool,
    data: Option<Value>,
    blocked: Option<bool>,
    error: Option<String>,
    status: Option<u16>,
}

#[derive(Deserialize)]
struct BrowserOpenCheckResult {
    ok: bool,
    data: Option<Value>,
    blocked: Option<bool>,
    error: Option<String>,
    status: Option<u16>,
}

// ── Public loaders ────────────────────────────────────────────────────────────

/// Fetch all coming-soon locations and their details using an already-authenticated
/// browser page. Does not launch or close Chrome — the caller owns the browser lifecycle.
pub async fn load_from_browser(
    country: &str,
    page: &Page,
) -> Result<LoadResult, Box<dyn std::error::Error>> {
    tracing::info!("fetching location data from inside the browser");
    let json_text: String = page
        .evaluate(format!(
            "fetch('/api/findus/get-locations?country={country}').then(r => r.text())"
        ))
        .await?
        .into_value()?;

    if json_text.trim_start().starts_with('<') {
        tracing::error!("got HTML response — Akamai still blocking (try --show-browser to debug)");
        return Err("API returned HTML (access denied)".into());
    }

    let resp: ApiResponse = serde_json::from_str(&json_text)?;
    let locations = resp.data.data;
    let ids = coming_soon_ids(&locations);
    let total = ids.len();

    let num_batches = ids.chunks(DETAILS_BATCH_SIZE).count();
    tracing::info!(
        total,
        num_batches,
        batch_size = DETAILS_BATCH_SIZE,
        timeout_secs = DETAILS_TIMEOUT_SECS,
        "fetching details for coming-soon/winner superchargers"
    );

    let (coming_soon_details, failed_detail_ids, unknown_enum_tracker) =
        fetch_batch_details_from_page(page, ids).await;

    tracing::info!(
        resolved = coming_soon_details.len(),
        total,
        "details fetch complete"
    );

    Ok(LoadResult {
        locations,
        coming_soon_details,
        failed_detail_ids,
        unknown_enum_tracker,
    })
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
    let batches: Vec<&[String]> = ids.chunks(OPEN_STATUS_BATCH_SIZE).collect();
    let num_batches = batches.len();

    for (i, batch) in batches.iter().enumerate() {
        let batch_json = serde_json::to_string(batch)?;

        let text: String = page
            .evaluate(format!(
                r#"
                (() => {{
                    const slugs = {batch_json};
                    return Promise.all(
                        slugs.map(slug =>
                            fetch(`/api/findus/get-location-details?locationSlug=${{slug}}&functionTypes=supercharger&locale=en_US&isInHkMoTw=false`,
                                  {{ signal: AbortSignal.timeout({timeout_ms}) }})
                                .then(async r => {{
                                    const text = await r.text();
                                    return {{r, text}};
                                }})
                                .then(({{r, text}}) => {{
                                    const status = r.status;
                                    const trimmed = text.trimStart();
                                    if (trimmed.startsWith('<')) {{
                                        return {{ok: false, data: null, blocked: true, error: 'html_block', status}};
                                    }}
                                    if (!r.ok) {{
                                        return {{ok: false, data: null, blocked: false, error: 'http_error', status}};
                                    }}
                                    try {{
                                        return {{ok: true, data: JSON.parse(text), blocked: false, error: null, status}};
                                    }} catch (_) {{
                                        return {{ok: false, data: null, blocked: false, error: 'json_parse', status}};
                                    }}
                                }})
                                .catch(error => ({{
                                    ok: false,
                                    data: null,
                                    blocked: false,
                                    error: error?.name === 'TimeoutError' ? 'timeout' : 'fetch_failed',
                                    status: null
                                }}))
                        )
                    ).then(results => JSON.stringify(slugs.map((s, i) => [s, results[i]])));
                }})()
                "#
            ))
            .await?
            .into_value()?;

        let pairs: Vec<(String, BrowserOpenCheckResult)> = serde_json::from_str(&text)?;
        let blocked = pairs
            .iter()
            .any(|(_, result)| result.blocked.unwrap_or(false));

        for (id, result) in pairs {
            if !result.ok {
                if open_check_not_found(&result) {
                    tracing::info!(
                        id,
                        status = result.status,
                        "open-check endpoint returned not found — treating as checked absent"
                    );
                    continue;
                }

                tracing::warn!(
                    id,
                    reason = open_failure_reason(&result),
                    status = result.status,
                    "open-check fetch failed — flagging for retry"
                );
                failed.insert(id);
                continue;
            }
            let Some(data) = result.data else { continue };
            let Ok(resp) = serde_json::from_value::<OpenCheckResponse>(data) else {
                tracing::warn!(
                    id,
                    "open-check response schema did not match expected shape — flagging for retry"
                );
                failed.insert(id);
                continue;
            };
            let Some(sf) = resp.data.supercharger_function else {
                continue;
            };
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

            results.insert(
                id,
                OpenResult {
                    opening_date,
                    num_stalls,
                    open_to_non_tesla: sf.open_to_non_tesla,
                    installed_full_power_kw: parse_installed_full_power_kw(
                        sf.installed_full_power.as_deref(),
                    ),
                },
            );
        }

        if blocked {
            tracing::warn!(
                batch = i + 1,
                num_batches,
                "open-status endpoint returned HTML block page — aborting remaining checks"
            );
            failed.extend(batches[i + 1..].iter().flat_map(|b| b.iter().cloned()));
            break;
        }

        if i + 1 < num_batches {
            tokio::time::sleep(Duration::from_millis(DETAILS_BATCH_DELAY_MS)).await;
        }
    }

    Ok((results, failed))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_installed_full_power_kw(raw: Option<&str>) -> Option<i32> {
    let kw = raw?.parse::<i32>().ok()?;
    if kw == 0 { None } else { Some(kw) }
}

/// Collect IDs (Tesla location URL slugs) for all coming-soon superchargers that have one.
fn coming_soon_ids(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .filter(|l| {
            l.location_type.iter().any(|t| {
                matches!(
                    t.as_str(),
                    "coming_soon_supercharger"
                        | "winner_supercharger"
                        | "current_winner_supercharger"
                )
            })
        })
        .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
        .map(|l| l.location_url_slug.clone())
        .collect()
}

/// Launch Chrome (headless or visible), navigate to Tesla.com, and wait for Akamai cookies.
/// Returns the browser handle and the ready page — caller is responsible for closing the browser.
pub async fn launch_browser_and_wait(
    show_browser: bool,
) -> Result<BrowserSession, Box<dyn std::error::Error>> {
    let chrome = find_chrome()?;
    let profile_dir = tempfile::Builder::new()
        .prefix("tesla-superchargers-chrome-")
        .tempdir()?;

    tracing::info!(headless = !show_browser, "launching Chrome");
    tracing::debug!(
        profile_dir = %profile_dir.path().display(),
        "using fresh Chrome profile"
    );

    let browser_args = [
        "--no-first-run",
        "--no-default-browser-check",
        "--window-size=1280,800",
    ];

    let config = if show_browser {
        let mut b = BrowserConfig::builder()
            .chrome_executable(&chrome)
            .disable_default_args()
            .hide()
            .user_data_dir(profile_dir.path())
            .with_head();
        for arg in &browser_args {
            b = b.arg(*arg);
        }
        b.build()?
    } else {
        let mut b = BrowserConfig::builder()
            .chrome_executable(&chrome)
            .disable_default_args()
            .hide();
        b = b.user_data_dir(profile_dir.path());
        for arg in &browser_args {
            b = b.arg(*arg);
        }
        b.build()?
    };

    let (browser, mut handler) = Browser::launch(config).await?;

    tokio::spawn(async move { while handler.next().await.is_some() {} });

    // Open a blank page first — passing a URL to new_page() makes chromiumoxide
    // wait for the load event, which Akamai can block indefinitely.
    let page = browser.new_page("about:blank").await?;
    tracing::info!("navigating to https://www.tesla.com/findus");
    page.goto("https://www.tesla.com/findus").await?;

    wait_for_akamai_api_access(&page, show_browser).await?;

    Ok(BrowserSession {
        browser,
        page,
        _profile_dir: profile_dir,
    })
}

async fn wait_for_akamai_api_access(
    page: &Page,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let timeout_secs = if show_browser {
        VISIBLE_AKAMAI_TIMEOUT_SECS
    } else {
        HEADLESS_AKAMAI_TIMEOUT_SECS
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    tracing::info!(
        timeout_secs,
        "waiting for Tesla API access (Akamai cookies)"
    );
    if show_browser {
        tracing::info!("if Tesla shows a browser challenge, complete it in the Chrome window");
    }

    tracing::debug!(
        settle_secs = AKAMAI_INITIAL_SETTLE_SECS,
        "letting Tesla/Akamai scripts settle before probing API access"
    );
    tokio::time::sleep(Duration::from_secs(AKAMAI_INITIAL_SETTLE_SECS)).await;

    let mut last_log = Instant::now();
    let mut poll_delay = Duration::from_secs(if show_browser { 10 } else { 5 });
    while Instant::now() < deadline {
        match tesla_api_access_status(page).await {
            TeslaApiAccessStatus::Ready => {
                tracing::info!("Tesla API access is ready");
                return Ok(());
            }
            status => {
                if show_browser && last_log.elapsed() >= Duration::from_secs(15) {
                    tracing::info!(
                        status = status.as_str(),
                        "still waiting for Tesla API access"
                    );
                    last_log = Instant::now();
                }
            }
        }
        tokio::time::sleep(poll_delay).await;
        poll_delay = (poll_delay + Duration::from_secs(5)).min(Duration::from_secs(20));
    }

    Err(format!("Tesla API still blocked after {timeout_secs}s").into())
}

enum TeslaApiAccessStatus {
    Ready,
    HtmlBlocked,
    UnexpectedJson,
    FetchFailed,
    EvaluateFailed,
}

impl TeslaApiAccessStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::HtmlBlocked => "html_blocked",
            Self::UnexpectedJson => "unexpected_json",
            Self::FetchFailed => "fetch_failed",
            Self::EvaluateFailed => "evaluate_failed",
        }
    }
}

async fn tesla_api_access_status(page: &Page) -> TeslaApiAccessStatus {
    let Ok(value) = page
        .evaluate(
            r#"
            fetch('/api/findus/get-locations?country=US')
                .then(r => r.text())
                .then(text => {
                    if (text.trimStart().startsWith('<')) return 'html_blocked';
                    try {
                        const json = JSON.parse(text);
                        return Array.isArray(json?.data?.data) ? 'ready' : 'unexpected_json';
                    } catch (_) {
                        return 'unexpected_json';
                    }
                })
                .catch(() => 'fetch_failed')
            "#,
        )
        .await
    else {
        return TeslaApiAccessStatus::EvaluateFailed;
    };

    match value.into_value::<String>().ok().as_deref() {
        Some("ready") => TeslaApiAccessStatus::Ready,
        Some("html_blocked") => TeslaApiAccessStatus::HtmlBlocked,
        Some("unexpected_json") => TeslaApiAccessStatus::UnexpectedJson,
        Some("fetch_failed") => TeslaApiAccessStatus::FetchFailed,
        _ => TeslaApiAccessStatus::EvaluateFailed,
    }
}

/// Fetch details for `ids` in batches from an already-authenticated browser page.
/// Returns `(details_map, failed_ids, unknown_enum_tracker)`. Retries failed IDs once before returning.
pub async fn fetch_batch_details_from_page(
    page: &Page,
    ids: Vec<String>,
) -> (
    HashMap<String, ComingSoonDetails>,
    HashSet<String>,
    UnknownEnumTracker,
) {
    let batches: Vec<&[String]> = ids.chunks(DETAILS_BATCH_SIZE).collect();
    let num_batches = batches.len();
    let timeout_ms = DETAILS_TIMEOUT_SECS * 1000;

    let mut details: HashMap<String, ComingSoonDetails> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    let mut unknown_enum_tracker = UnknownEnumTracker::default();

    for (i, batch) in batches.iter().enumerate() {
        tracing::info!(
            batch = i + 1,
            num_batches,
            size = batch.len(),
            "fetching detail batch"
        );
        let result = fetch_detail_batch_from_page_with_timeout(
            page,
            batch,
            timeout_ms,
            &mut unknown_enum_tracker,
        )
        .await;
        details.extend(result.details);
        failed.extend(result.failed_ids);

        if result.blocked {
            tracing::warn!(
                batch = i + 1,
                num_batches,
                "detail endpoint returned block response — aborting remaining batches"
            );
            failed.extend(batches[i + 1..].iter().flat_map(|b| b.iter().cloned()));
            break;
        }

        if i + 1 < num_batches {
            tokio::time::sleep(Duration::from_millis(DETAILS_BATCH_DELAY_MS)).await;
        }
    }

    tracing::info!(
        attempted = ids.len(),
        resolved = ids.len().saturating_sub(failed.len()),
        with_details = details.len(),
        failed = failed.len(),
        "detail fetch summary"
    );

    (details, failed, unknown_enum_tracker)
}

pub async fn fetch_detail_batch_from_page(
    page: &Page,
    ids: &[String],
    unknown_enums: &mut UnknownEnumTracker,
) -> DetailBatchFetchResult {
    fetch_detail_batch_from_page_with_timeout(page, ids, DETAILS_TIMEOUT_SECS * 1000, unknown_enums)
        .await
}

async fn fetch_detail_batch_from_page_with_timeout(
    page: &Page,
    ids: &[String],
    timeout_ms: u64,
    unknown_enums: &mut UnknownEnumTracker,
) -> DetailBatchFetchResult {
    let mut result = fetch_detail_batch_once(page, ids, timeout_ms, unknown_enums).await;
    log_detail_batch_result("detail batch result", ids.len(), &result);

    if result.blocked || result.failed_ids.is_empty() {
        return result;
    }

    let retry_ids: Vec<String> = result.failed_ids.iter().cloned().collect();
    tracing::warn!(
        count = retry_ids.len(),
        "detail batch failed — retrying once"
    );
    tokio::time::sleep(Duration::from_millis(DETAILS_FAILURE_BACKOFF_MS)).await;

    let retry_result = fetch_detail_batch_once(page, &retry_ids, timeout_ms, unknown_enums).await;
    log_detail_batch_result("detail retry batch result", retry_ids.len(), &retry_result);

    result.details.extend(retry_result.details);
    result.resolved_without_details += retry_result.resolved_without_details;
    result.failed_ids = retry_result.failed_ids;
    result.failure_reasons = retry_result.failure_reasons;
    result.blocked = retry_result.blocked;
    result
}

async fn fetch_detail_batch_once(
    page: &Page,
    ids: &[String],
    timeout_ms: u64,
    unknown_enums: &mut UnknownEnumTracker,
) -> DetailBatchFetchResult {
    let batch_json = match serde_json::to_string(ids) {
        Ok(s) => s,
        Err(_) => {
            let mut result = DetailBatchFetchResult::default();
            result.failed_ids.extend(ids.iter().cloned());
            result
                .failure_reasons
                .insert("serialize_failed".into(), ids.len());
            return result;
        }
    };

    let Some(pairs) = eval_detail_batch(page, &batch_json, timeout_ms).await else {
        let mut result = DetailBatchFetchResult::default();
        result.failed_ids.extend(ids.iter().cloned());
        result
            .failure_reasons
            .insert("evaluate_failed".into(), ids.len());
        return result;
    };

    classify_detail_pairs(pairs, unknown_enums)
}

fn classify_detail_pairs(
    pairs: Vec<(String, BrowserDetailResult)>,
    unknown_enums: &mut UnknownEnumTracker,
) -> DetailBatchFetchResult {
    let mut result = DetailBatchFetchResult::default();
    result.blocked = pairs
        .iter()
        .any(|(_, result)| result.blocked.unwrap_or(false));

    for (id, browser_result) in pairs {
        if browser_result.ok {
            match detail_from_value(browser_result.data, unknown_enums) {
                Ok(Some(details)) => {
                    result.details.insert(id, details);
                }
                Ok(None) => {
                    result.resolved_without_details += 1;
                }
                Err(reason) => {
                    *result.failure_reasons.entry(reason).or_insert(0) += 1;
                    result.failed_ids.insert(id);
                }
            }
        } else {
            *result
                .failure_reasons
                .entry(detail_failure_reason(&browser_result))
                .or_insert(0) += 1;
            result.failed_ids.insert(id);
        }
    }

    result
}

fn log_detail_batch_result(label: &'static str, attempted: usize, result: &DetailBatchFetchResult) {
    tracing::info!(
        attempted,
        resolved = result.resolved_count(attempted),
        with_details = result.details.len(),
        resolved_without_details = result.resolved_without_details,
        failed = result.failed_ids.len(),
        blocked = result.blocked,
        reasons = ?result.failure_reasons,
        label
    );
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
                        fetch(`/api/findus/get-location-details?locationSlug=${{slug}}&functionTypes=coming_soon_supercharger,supercharger&locale=en_US&isInHkMoTw=false`,
                              {{ signal: AbortSignal.timeout({timeout_ms}) }})
                            .then(async r => {{
                                const text = await r.text();
                                return {{r, text}};
                            }})
                            .then(({{r, text}}) => {{
                                const status = r.status;
                                const trimmed = text.trimStart();
                                if (trimmed.startsWith('<')) {{
                                    return {{ok: false, data: null, blocked: true, error: 'html_block', status}};
                                }}
                                if (!r.ok) {{
                                    return {{ok: false, data: null, blocked: false, error: 'http_error', status}};
                                }}
                                try {{
                                    return {{ok: true, data: JSON.parse(text), blocked: false, error: null, status}};
                                }} catch (_) {{
                                    return {{ok: false, data: null, blocked: false, error: 'json_parse', status}};
                                }}
                            }})
                            .catch(error => ({{
                                ok: false,
                                data: null,
                                blocked: false,
                                error: error?.name === 'TimeoutError' ? 'timeout' : 'fetch_failed',
                                status: null
                            }}))
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

fn detail_failure_reason(result: &BrowserDetailResult) -> String {
    let reason = result
        .error
        .as_deref()
        .unwrap_or(if result.blocked.unwrap_or(false) {
            "html_block"
        } else {
            "unknown"
        });

    match result.status {
        Some(status) => format!("{reason}:{status}"),
        None => reason.to_string(),
    }
}

fn non_empty_opt(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
}

fn detail_from_value(
    data: Option<Value>,
    tracker: &mut UnknownEnumTracker,
) -> Result<Option<ComingSoonDetails>, String> {
    let Some(data) = data else {
        return Ok(None);
    };
    let resp: LocationDetailsResponse =
        serde_json::from_value(data).map_err(|_| "schema_mismatch".to_string())?;

    let Some(sf) = resp.data.supercharger_function else {
        return Ok(None);
    };

    if let Some(ref value) = sf.project_status
        && !value.is_empty()
    {
        tracker.record("project_status", value, KNOWN_PROJECT_STATUS);
    }
    if let Some(ref value) = sf.charging_accessibility
        && !value.is_empty()
    {
        tracker.record(
            "charging_accessibility",
            value,
            KNOWN_CHARGING_ACCESSIBILITY,
        );
    }

    let address = resp
        .data
        .functions
        .as_ref()
        .and_then(|functions| functions.first())
        .and_then(|function| function.address.as_ref());

    Ok(Some(ComingSoonDetails {
        customer_facing_coming_soon_date: sf.customer_facing_coming_soon_date,
        coming_soon_name: sf.coming_soon_name,
        project_status: non_empty_opt(sf.project_status),
        num_charger_stalls: sf.num_charger_stalls,
        charging_accessibility: non_empty_opt(sf.charging_accessibility),
        street_address: address.and_then(|a| non_empty_opt(a.address_1.clone())),
        county: address.and_then(|a| non_empty_opt(a.county.clone())),
        postal_code: address.and_then(|a| non_empty_opt(a.postal_code.clone())),
        country_code: address.and_then(|a| non_empty_opt(a.country.clone())),
    }))
}

fn open_failure_reason(result: &BrowserOpenCheckResult) -> &str {
    result
        .error
        .as_deref()
        .unwrap_or(if result.blocked.unwrap_or(false) {
            "html_block"
        } else {
            "unknown"
        })
}

fn open_check_not_found(result: &BrowserOpenCheckResult) -> bool {
    result.status == Some(404)
        && result.error.as_deref() == Some("http_error")
        && !result.blocked.unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open_result(
        error: Option<&str>,
        status: Option<u16>,
        blocked: Option<bool>,
    ) -> BrowserOpenCheckResult {
        BrowserOpenCheckResult {
            ok: false,
            data: None,
            blocked,
            error: error.map(str::to_string),
            status,
        }
    }

    #[test]
    fn open_check_404_is_checked_absent_not_retryable() {
        let result = open_result(Some("http_error"), Some(404), Some(false));

        assert!(open_check_not_found(&result));
    }

    #[test]
    fn open_check_non_404_http_error_still_retries() {
        let result = open_result(Some("http_error"), Some(500), Some(false));

        assert!(!open_check_not_found(&result));
    }

    #[test]
    fn open_check_html_block_with_404_still_retries() {
        let result = open_result(Some("html_block"), Some(404), Some(true));

        assert!(!open_check_not_found(&result));
    }

    #[test]
    fn detail_from_value_merges_supercharger_function_and_address() {
        let payload = json!({
            "data": {
                "supercharger_function": {
                    "customer_facing_coming_soon_date": "In Development",
                    "coming_soon_name": "Padova, Italy",
                    "project_status": "Design",
                    "num_charger_stalls": "8",
                    "charging_accessibility": "Tesla Only"
                },
                "functions": [{
                    "address": {
                        "address_1": "5 Via Sergio Fraccalanza",
                        "county": "Provincia di Padova",
                        "postal_code": "35129",
                        "country": "IT"
                    }
                }]
            }
        });

        let mut tracker = UnknownEnumTracker::default();
        let details = detail_from_value(Some(payload), &mut tracker)
            .unwrap()
            .unwrap();

        assert_eq!(details.project_status.as_deref(), Some("Design"));
        assert_eq!(details.num_charger_stalls.as_deref(), Some("8"));
        assert_eq!(
            details.street_address.as_deref(),
            Some("5 Via Sergio Fraccalanza")
        );
        assert_eq!(details.country_code.as_deref(), Some("IT"));
        assert_eq!(tracker.count("project_status", "Mystery"), 0);
    }

    #[test]
    fn detail_from_value_without_supercharger_function_is_none() {
        let payload = json!({
            "data": {
                "functions": [{
                    "address": { "address_1": "123 Main St", "country": "US" }
                }]
            }
        });

        let mut tracker = UnknownEnumTracker::default();
        assert!(
            detail_from_value(Some(payload), &mut tracker)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn parse_installed_full_power_kw_maps_values() {
        assert_eq!(parse_installed_full_power_kw(Some("250")), Some(250));
        assert_eq!(parse_installed_full_power_kw(Some("0")), None);
        assert_eq!(parse_installed_full_power_kw(None), None);
        assert_eq!(parse_installed_full_power_kw(Some("not-a-number")), None);
    }

    #[test]
    fn unknown_enum_tracker_warns_once_and_counts() {
        let mut tracker = UnknownEnumTracker::default();
        tracker.record("project_status", "Mystery", KNOWN_PROJECT_STATUS);
        tracker.record("project_status", "Mystery", KNOWN_PROJECT_STATUS);
        tracker.record(
            "charging_accessibility",
            "Future Cars",
            KNOWN_CHARGING_ACCESSIBILITY,
        );

        assert_eq!(tracker.count("project_status", "Mystery"), 2);
        assert_eq!(tracker.count("charging_accessibility", "Future Cars"), 1);
    }
}
