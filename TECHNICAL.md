# Tesla Supercharger Fetcher — Technical Documentation

## Overview

A Rust CLI tool that fetches Tesla's full global location dataset and filters it to display planned and open Superchargers. The tool reverse-engineers Tesla's internal Find Us API and works around Akamai Bot Manager using a headless Chrome browser controlled via the Chrome DevTools Protocol (CDP).

---

## Project Structure

```
tesla-superchargers/
├── Cargo.toml          # Dependencies
├── src/
│   └── main.rs         # All application code
├── TECHNICAL.md        # This file
└── tesla-supercharger-api.md  # API research notes
```

---

## The API

### Endpoint

```
GET https://www.tesla.com/api/findus/get-locations
```

Discovered by inspecting network traffic on `https://www.tesla.com/findus` and searching the Next.js JS bundle (`pages/[locale]/findus-*.js`) for the string `get-locations`.

### The `country` parameter

| Request | Total results | Location types |
|---|---|---|
| No params | ~5,071 | Open superchargers only |
| `?country=US` | ~21,450 | All types worldwide |

Despite the name, `country=US` is **not a geographic filter** — it's a mode switch that tells the server to return the full multi-type dataset. All countries are included in the response.

### Response structure

```json
{
  "data": {
    "data": [
      {
        "uuid": "11399610",
        "title": "Highbridge, United Kingdom",
        "latitude": 51.22962,
        "longitude": -2.959685,
        "location_type": ["coming_soon_supercharger"],
        "location_url_slug": "11255",
        "coming_soon_supercharger": true
      }
    ]
  }
}
```

### Location types

All `location_type` values observed in the full dataset:

| Value | Description |
|---|---|
| `supercharger` | Open Supercharger |
| `coming_soon_supercharger` | **Planned Supercharger** |
| `megacharger` | Open Megacharger |
| `coming_soon_megacharger` | Planned Megacharger |
| `destination_charger` | Destination charger (Tesla) |
| `destination_charger_nontesla` | Non-Tesla destination charger |
| `nacs` | NACS adapter compatible |
| `service` | Service centre |
| `coming_soon_service` | Planned service centre |
| `sales` | Sales location |
| `delivery_center` | Delivery centre |
| `gallery` | Showroom/gallery |
| `bodyshop` | Body shop |
| `vendor_collision` | Vendor collision centre |
| `self_serve_demo_drive` | Self-serve demo drive |
| `party` | Tesla event |
| `winner_supercharger` | Supercharger voting winner |
| `current_winner_supercharger` | Current Supercharger vote winner |

### Planned vs. open object shapes

Planned Supercharger objects are minimal — no `supercharger_function` block:

```json
{
  "coming_soon_supercharger": true,
  "uuid": "11399610",
  "title": "Highbridge, United Kingdom",
  "latitude": 51.22962,
  "longitude": -2.959685,
  "location_type": ["coming_soon_supercharger"],
  "location_url_slug": "11255"
}
```

Open Supercharger objects include a `supercharger_function` block with operational details:

```json
{
  "uuid": "...",
  "title": "...",
  "latitude": 64.250138,
  "longitude": -15.205539,
  "location_type": ["supercharger"],
  "supercharger_function": {
    "access_type": "Public",
    "open_to_non_tesla": true,
    "site_status": "open",
    "project_status": "Open",
    "charging_accessibility": "All Vehicles (Production)"
  }
}
```

---

## Why a Plain HTTP Client Doesn't Work

Tesla's API is protected by **Akamai Bot Manager**, which performs multi-layer bot detection:

### Layer 1 — JavaScript Challenge
When a browser first loads `tesla.com`, Akamai injects JavaScript that:
- Runs behavioural analysis (mouse movement, timing, canvas fingerprinting)
- Generates a cryptographic sensor token
- Sets several cookies including `_abck` and `bm_sz`

Without executing this JavaScript, no valid session cookies are set, and the API returns `403 Access Denied`.

### Layer 2 — TLS Fingerprinting (JA3)
Even if you copy the browser's cookies into a curl/reqwest request, Akamai compares the **JA3 hash** of the TLS ClientHello against the cookies' origin browser. A Rust `reqwest` client has a different TLS fingerprint than Chrome, so the fingerprint-to-cookie binding fails and the request is still blocked.

### What this means in practice

| Approach | Result | Why |
|---|---|---|
| Plain `curl` | `403` | No session cookies |
| `curl` with browser `document.cookie` | `403` | Missing HttpOnly `_abck` cookie |
| `reqwest` with all CDP-extracted cookies | `403` | JA3 mismatch |
| Fetch **from inside the browser** | ✅ `200` | Correct TLS fingerprint + cookies |

---

## The Solution: CDP In-Browser Fetch

The tool uses **chromiumoxide** (a Rust CDP client) to:

1. Launch a real Chrome instance (headless or visible)
2. Navigate to `https://www.tesla.com/findus`, allowing Akamai's JS challenge to complete naturally
3. Wait 6 seconds for all session cookies to be set
4. Execute `fetch('/api/findus/get-locations?country=US').then(r => r.text())` **from within the page context** via `Runtime.evaluate`
5. The fetch originates from Chrome itself — correct TLS fingerprint, correct cookies, Akamai satisfied
6. The JSON text is returned to Rust via CDP and deserialized

```
┌─────────────────────────────────────────────────────────────────┐
│  Rust (tesla-superchargers binary)                              │
│                                                                 │
│   chromiumoxide ──CDP──▶ Chrome (headless)                      │
│       │                      │                                  │
│       │              navigate to /findus                        │
│       │              Akamai JS runs, cookies set                │
│       │                      │                                  │
│       │◀── evaluate() ───────┤                                  │
│       │    fetch('/api/...')  │──HTTPS──▶ tesla.com API         │
│       │                      │◀── 200 JSON ────────────────────│
│       │◀── JSON text ────────┤                                  │
│       │                                                         │
│   serde_json::from_str()                                        │
│   filter + print table                                          │
└─────────────────────────────────────────────────────────────────┘
```

### Stealth flags

Chrome is launched with flags that suppress automation signals Akamai looks for:

```
--disable-blink-features=AutomationControlled   ← hides navigator.webdriver = true
--excludeSwitches=enable-automation             ← removes automation banner
--no-first-run                                  ← suppress first-run UI
--disable-extensions                            ← cleaner environment
--window-size=1280,800                          ← realistic viewport
```

---

## Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `tokio` | 1 (full) | Async runtime |
| `serde` + `serde_json` | 1 | JSON deserialization |
| `reqwest` | 0.12 | HTTP client (used in `--cookie` mode) |
| `clap` | 4 | CLI argument parsing with env var support |
| `chromiumoxide` | 0.7 | Chrome DevTools Protocol client |
| `futures` | 0.3 | `StreamExt` for driving the CDP handler stream |

---

## Data Flow & Modes

The tool supports three data loading modes, checked in priority order:

```
cargo run
  │
  ├─ --file PATH          → load_from_file()      reads local JSON, no network
  │
  ├─ --cookie STR         → load_with_cookie()    plain reqwest (needs valid
  │   (or TESLA_COOKIE)                           Akamai session cookies)
  │
  └─ (default)            → load_from_browser()   launches Chrome via CDP,
                                                   fetches from inside the page
```

### `load_from_file`
Reads a JSON file exported from the browser. Fast, offline, no auth required.
```bash
cargo run -- --file ~/Downloads/tesla_locations.json
```

### `load_with_cookie`
Direct reqwest call with a manually provided cookie string. Will fail against a live Tesla endpoint due to TLS fingerprinting, but kept for use with proxies or alternative environments where fingerprinting is not enforced.
```bash
export TESLA_COOKIE="bm_sz=...; _abck=..."
cargo run
```

### `load_from_browser` (default)
Launches Chrome via chromiumoxide, navigates to Tesla Find Us, waits for Akamai to complete, then runs the API fetch inside the browser context.
```bash
cargo run                       # headless (no window)
cargo run -- --show-browser     # visible window for debugging
```

---

## CLI Reference

```
USAGE:
    tesla-superchargers [OPTIONS]

OPTIONS:
    -f, --file <PATH>           Read from a local JSON file
    -c, --cookie <COOKIE_STR>   Session cookie string [env: TESLA_COOKIE]
        --country <CODE>        API country code [default: US]
        --show-browser          Show Chrome window (default: headless)
        --show-open             Also print table of open superchargers
    -h, --help                  Print help
    -V, --version               Print version
```

---

## Refreshing Data

To get fresh data without running the Rust tool, paste this into the Chrome DevTools console on `https://www.tesla.com/findus`:

```js
fetch('/api/findus/get-locations?country=US')
  .then(r => r.json())
  .then(d => {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(
      new Blob([JSON.stringify(d)], { type: 'application/json' })
    );
    a.download = 'tesla_locations.json';
    a.click();
  });
```

Then use: `cargo run -- --file ~/Downloads/tesla_locations.json`

---

## Known Limitations

- **No construction stage data** — planned Supercharger objects (`coming_soon_supercharger`) do not include a project stage (permit / under construction / etc.). That detail is not exposed by this endpoint.
- **Some entries have `"locations"` as the title** — a data quality issue on Tesla's side. These entries have valid coordinates but a generic placeholder name.
- **Akamai timing sensitivity** — the 6-second wait after page load is a heuristic. On slow connections it may need increasing.
- **Chrome required** — the default mode needs Google Chrome or Chromium installed. Checked paths: macOS app bundle, `/usr/bin/google-chrome`, `/usr/bin/chromium`, `/usr/bin/chromium-browser`.
- **Live data ~9.5 MB** — the full `?country=US` response is ~9.5 MB of JSON (~21,450 locations).
