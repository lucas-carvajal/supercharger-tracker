use chromiumoxide::{Browser, BrowserConfig};
use clap::Parser;
use futures::StreamExt;
use serde::Deserialize;
use std::{fs, time::Duration};

const API_URL: &str = "https://www.tesla.com/api/findus/get-locations";

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "tesla-superchargers",
    version,
    about = "Fetch Tesla Supercharger locations"
)]
struct Args {
    /// Read from a local JSON file instead of fetching live data.
    #[arg(short, long, value_name = "PATH")]
    file: Option<String>,

    /// Use a raw cookie string instead of launching a browser.
    /// Can also be set via TESLA_COOKIE env var.
    #[arg(short, long, value_name = "COOKIE_STRING", env = "TESLA_COOKIE")]
    cookie: Option<String>,

    /// Country code (default: US — actually returns worldwide data).
    #[arg(long, default_value = "US")]
    country: String,

    /// Show the browser window while fetching (default: headless).
    #[arg(long)]
    show_browser: bool,

    /// Also print the table of open superchargers.
    #[arg(long)]
    show_open: bool,
}

// ── API response shapes ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ApiResponse {
    data: DataWrapper,
}

#[derive(Deserialize)]
struct DataWrapper {
    data: Vec<Location>,
}

#[derive(Deserialize)]
struct Location {
    #[allow(dead_code)]
    uuid: String,
    title: String,
    latitude: f64,
    longitude: f64,
    location_type: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    supercharger_function: Option<SuperchargerFunction>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SuperchargerFunction {
    access_type: Option<String>,
    open_to_non_tesla: Option<bool>,
    site_status: Option<String>,
    charging_accessibility: Option<String>,
}

// ── data loading ──────────────────────────────────────────────────────────────

async fn load_from_file(path: &str) -> Result<Vec<Location>, Box<dyn std::error::Error>> {
    println!("Reading from file: {path}");
    let raw = fs::read_to_string(path)?;
    let resp: ApiResponse = serde_json::from_str(&raw)?;
    Ok(resp.data.data)
}

async fn load_with_cookie(
    country: &str,
    cookie: &str,
) -> Result<Vec<Location>, Box<dyn std::error::Error>> {
    let url = format!("{API_URL}?country={country}");
    println!("Fetching via HTTP: {url}");

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
            .parse()?,
    );
    headers.insert(
        reqwest::header::REFERER,
        "https://www.tesla.com/findus".parse()?,
    );
    headers.insert(reqwest::header::ACCEPT, "application/json".parse()?);
    headers.insert(reqwest::header::COOKIE, cookie.parse()?);

    let response = reqwest::Client::builder()
        .default_headers(headers)
        .build()?
        .get(&url)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() || !body.trim_start().starts_with('{') {
        eprintln!("  ✗ HTTP {status}");
        eprintln!("  ✗ Response: {}", &body[..body.len().min(300)]);
        return Err(format!("API returned non-JSON response (HTTP {status})").into());
    }

    let resp: ApiResponse = serde_json::from_str(&body)?;
    Ok(resp.data.data)
}

async fn load_from_browser(
    country: &str,
    show_browser: bool,
) -> Result<Vec<Location>, Box<dyn std::error::Error>> {
    let chrome = find_chrome()?;

    println!(
        "Launching Chrome ({})…",
        if show_browser { "visible" } else { "headless" }
    );

    // --disable-blink-features=AutomationControlled hides navigator.webdriver = true
    // which Akamai uses as a primary bot-detection signal
    let stealth_args = [
        "--no-first-run",
        "--disable-extensions",
        "--disable-blink-features=AutomationControlled",
        "--excludeSwitches=enable-automation",
        "--window-size=1280,800",
    ];

    let config = if show_browser {
        let mut b = BrowserConfig::builder()
            .chrome_executable(&chrome)
            .with_head();
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

    // Drive the CDP connection in the background
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    println!("  → Opening https://www.tesla.com/findus");
    let page = browser.new_page("https://www.tesla.com/findus").await?;
    page.wait_for_navigation().await?;

    // Give Akamai's JS challenge time to complete and set cookies
    println!("  → Waiting for session cookies (Akamai)…");
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Make the API call from *inside* the browser so Chrome's own TLS
    // fingerprint + Akamai cookies are used. Using reqwest externally would
    // be blocked even with the right cookies due to JA3 fingerprint mismatch.
    println!("  → Fetching location data from inside the browser…");
    let json_text: String = page
        .evaluate(format!(
            "fetch('/api/findus/get-locations?country={country}').then(r => r.text())"
        ))
        .await?
        .into_value()?;

    browser.close().await.ok();

    if json_text.trim_start().starts_with('<') {
        eprintln!("  ✗ Got HTML — Akamai still blocking (try --show-browser to debug)");
        return Err("API returned HTML (access denied)".into());
    }

    let resp: ApiResponse = serde_json::from_str(&json_text)?;
    Ok(resp.data.data)
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

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let locations = if let Some(ref path) = args.file {
        load_from_file(path).await?
    } else if let Some(ref cookie) = args.cookie {
        load_with_cookie(&args.country, cookie).await?
    } else {
        load_from_browser(&args.country, args.show_browser).await?
    };

    let open: Vec<&Location> = locations
        .iter()
        .filter(|l| l.location_type.iter().any(|t| t == "supercharger"))
        .collect();

    let planned: Vec<&Location> = locations
        .iter()
        .filter(|l| {
            l.location_type
                .iter()
                .any(|t| t == "coming_soon_supercharger")
        })
        .collect();

    println!();
    println!("Total locations (all types) : {}", locations.len());
    println!("Open superchargers          : {}", open.len());
    println!("Planned superchargers       : {}", planned.len());

    if args.show_open {
        println!();
        print_table("OPEN SUPERCHARGERS", &open);
    }

    println!();
    print_table("PLANNED SUPERCHARGERS", &planned);

    println!("\nNote: country=US returns worldwide data — no need to repeat per country.");
    Ok(())
}

// ── display ───────────────────────────────────────────────────────────────────

fn print_table(title: &str, locations: &[&Location]) {
    println!("┌{:─<72}┐", "");
    println!("│ {:<70} │", title);
    println!("├{:─<5}┬{:─<43}┬{:─<11}┬{:─<9}┤", "", "", "", "");
    println!(
        "│ {:>3} │ {:<41} │ {:>9} │ {:>7} │",
        "#", "Name", "Lat", "Lon"
    );
    println!("├{:─<5}┼{:─<43}┼{:─<11}┼{:─<9}┤", "", "", "", "");
    for (i, loc) in locations.iter().enumerate() {
        println!(
            "│ {:>3} │ {:<41} │ {:>9.4} │ {:>7.4} │",
            i + 1,
            truncate(&loc.title, 41),
            loc.latitude,
            loc.longitude,
        );
    }
    println!("└{:─<5}┴{:─<43}┴{:─<11}┴{:─<9}┘", "", "", "", "");
    println!("  {} locations", locations.len());
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
