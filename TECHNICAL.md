# Tesla Supercharger Fetcher — Technical Documentation

## Overview

A Rust CLI tool that scrapes Tesla's internal Find Us API to track coming-soon Supercharger locations and persist them to Postgres. It filters the full global location dataset for planned Superchargers, fetches per-location status details, diffs against the current DB state, and writes the result in a single transaction. The tool reverse-engineers Tesla's internal Find Us API and works around Akamai Bot Manager using a headless Chrome browser controlled via the Chrome DevTools Protocol (CDP).

---

## Project Structure

```
tesla-superchargers/
├── Cargo.toml                       # Dependencies
├── .env.example                     # Environment variable template
├── migrations/
│   └── 20260327000000_init.sql      # DB schema (applied automatically on startup)
├── src/
│   ├── main.rs                      # CLI definition, startup, wiring
│   ├── coming_soon.rs               # ComingSoonSupercharger + SiteStatus types
│   ├── db.rs                        # Database access layer (connect, read, write)
│   ├── sync.rs                      # Pure diff logic (compute_sync, SyncPlan)
│   ├── display.rs                   # Terminal table rendering
│   ├── loaders.rs                   # Data loading (browser mode via CDP)
│   ├── raw.rs                       # Raw API deserialisation types
│   └── supercharger.rs              # Open supercharger type
├── TECHNICAL.md                     # This file
└── tesla-supercharger-api.md        # API research notes
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
│   filter + diff + persist                                       │
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
| `serde` + `serde_json` | 1 | JSON deserialisation |
| `clap` | 4 | CLI argument parsing with env var support |
| `chromiumoxide` | 0.7 | Chrome DevTools Protocol client |
| `futures` | 0.3 | `StreamExt` for driving the CDP handler stream |
| `sqlx` | 0.8 | Async Postgres driver; compile-time migrations via `sqlx::migrate!()` |
| `dotenvy` | 0.15 | `.env` file loading |

---

## Data Flow

### Per-run flow

```
startup
  │
  ├─ dotenvy::dotenv()          load .env
  ├─ db::connect()              connect to Postgres, run pending migrations
  │
  ├─ [scrape]
  │   ├─ load locations         (browser via CDP)
  │   ├─ filter coming_soon_supercharger entries
  │   └─ fetch per-location status details
  │
  ├─ db::record_scrape_run()    insert scrape_runs row, get run_id
  ├─ db::get_current_statuses() fetch all active chargers from DB
  ├─ sync::compute_sync()       diff DB state vs fresh scrape → SyncPlan
  └─ db::save_chargers()        persist SyncPlan in a single transaction
      ├─ bulk upsert new/changed chargers (unnest)
      ├─ touch last_scraped_at for unchanged chargers
      ├─ bulk insert status_changes rows (unnest)
      └─ mark disappeared chargers as is_active = FALSE
```

If `save_chargers` fails, the error is written back to the `scrape_runs` row before propagating.

### `load_from_browser`
Launches Chrome via chromiumoxide, navigates to Tesla Find Us, waits for Akamai to complete, then runs the API fetch inside the browser context.
```bash
cargo run -- scrape                # headless (no window)
cargo run -- scrape --show-browser # visible window for debugging
```

---

## Database Schema

### `scrape_runs`
One row per tool execution.

| Column | Type | Description |
|--------|------|-------------|
| `id` | `BIGSERIAL` | Primary key |
| `country` | `TEXT` | Country code passed to the API |
| `scraped_at` | `TIMESTAMPTZ` | When the run started |
| `total_count` | `INT` | Number of coming-soon chargers found |

### `coming_soon_superchargers`
One row per unique charger, keyed by Tesla's UUID.

| Column | Type | Description |
|--------|------|-------------|
| `uuid` | `TEXT` | Primary key (Tesla's UUID) |
| `title` | `TEXT` | Location name |
| `latitude` / `longitude` | `DOUBLE PRECISION` | Coordinates |
| `status` | `site_status` | Current status enum value |
| `location_url_slug` | `TEXT` | Slug used in Tesla's Find Us URL |
| `raw_status_value` | `TEXT` | Raw string from API before parsing |
| `first_seen_at` | `TIMESTAMPTZ` | When first observed in the feed |
| `last_scraped_at` | `TIMESTAMPTZ` | Last run where this charger was present |
| `is_active` | `BOOLEAN` | `FALSE` once absent from the feed |

### `status_changes`
Append-only audit log of status events.

| Column | Type | Description |
|--------|------|-------------|
| `id` | `BIGSERIAL` | Primary key |
| `supercharger_uuid` | `TEXT` | FK → `coming_soon_superchargers` |
| `scrape_run_id` | `BIGINT` | FK → `scrape_runs` |
| `old_status` | `site_status` | `NULL` = first sighting |
| `new_status` | `site_status` | Status observed in this run |
| `changed_at` | `TIMESTAMPTZ` | When the event was recorded |

### `site_status` enum
`IN_DEVELOPMENT` · `UNDER_CONSTRUCTION` · `UNKNOWN`

---

## CLI Reference

```
USAGE:
    tesla-superchargers <SUBCOMMAND>

SUBCOMMANDS:
    scrape          Fetch all coming-soon locations and update the DB
    status          Show a summary of the last run and current DB state
    retry-failed    Re-fetch details for chargers with failed detail fetches
    host            Start the read-only HTTP API server

scrape OPTIONS:
        --country <CODE>   API country code [default: US]
        --show-browser     Show Chrome window (default: headless)
    -h, --help             Print help

retry-failed OPTIONS:
        --show-browser     Show Chrome window (default: headless)
    -h, --help             Print help

host OPTIONS:
    -p, --port <PORT>      Port to listen on [default: 8080]
    -h, --help             Print help
```

---

## Known Limitations

- **No construction stage data** — planned Supercharger objects (`coming_soon_supercharger`) do not include a project stage (permit / under construction / etc.). That detail is not exposed by this endpoint.
- **Some entries have `"locations"` as the title** — a data quality issue on Tesla's side. These entries have valid coordinates but a generic placeholder name.
- **Akamai timing sensitivity** — the 6-second wait after page load is a heuristic. On slow connections it may need increasing.
- **Chrome required** — the default mode needs Google Chrome or Chromium installed. Checked paths: macOS app bundle, `/usr/bin/google-chrome`, `/usr/bin/chromium`, `/usr/bin/chromium-browser`.
- **Live data ~9.5 MB** — the full `?country=US` response is ~9.5 MB of JSON (~21,450 locations).
- **Disappearance ≠ opened** — a charger going inactive (`is_active = FALSE`) means it stopped appearing in the feed. It may have opened, been cancelled, or be a data issue. Use `last_scraped_at` to determine when it was last confirmed present.
- **`UNKNOWN` status transitions are recorded** — if the status details API returns null for a charger that previously had a real status, a `status_changes` row is written. This may occasionally produce noise.
