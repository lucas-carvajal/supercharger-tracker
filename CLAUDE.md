# CLAUDE.md

## Project Overview

`supercharger-tracker` is a Rust CLI tool that scrapes Tesla's internal Find Us API to track coming-soon Supercharger locations worldwide. It persists charger records and status transitions to PostgreSQL and exposes the data through a read-only HTTP API. The tool works around Akamai Bot Manager protection using headless Chrome via the Chrome DevTools Protocol (CDP).

## Tech Stack

| Crate | Role |
|---|---|
| **Rust** (edition 2024, stable) | Language |
| **Tokio** 1.x | Async runtime |
| **Axum** 0.8 | HTTP API server |
| **SQLx** 0.8 | Async Postgres driver with compile-time migrations |
| **Clap** 4.x | CLI argument parsing (derive macros, env var support) |
| **Chromiumoxide** 0.7 | CDP client for headless Chrome automation |
| **Reqwest** 0.12 | HTTP client (cookie/JSON support) |
| **Serde** / **serde_json** | JSON serialization |
| **Dotenvy** 0.15 | `.env` file loading |
| **Tower-http** 0.6 | CORS middleware |

## Setup

### Prerequisites
- Rust (stable) — install via [rustup.rs](https://rustup.rs)
- PostgreSQL 16 (compatible with 13+)
- Chrome or Chromium (required for default browser-based fetch mode)

### Environment Variables

Copy `.env.example` to `.env` and configure:

```bash
cp .env.example .env
```

| Variable | Required | Description |
|---|---|---|
| `DATABASE_URL` | Yes | Postgres connection string, e.g. `postgres://postgres:pass@localhost:5432/supercharger-db` |
| `TESLA_COOKIE` | No | Raw session cookie string for cookie-based fetch mode (alternative to browser mode) |

### Database

Migrations run automatically on startup via `sqlx::migrate!()`.

Optional quick start with Docker:
```bash
docker run -d --name supercharger-db \
  -e POSTGRES_DB=supercharger-db \
  -e POSTGRES_PASSWORD=pass \
  -p 5432:5432 postgres:16
```

## Build & Run

```bash
cargo build --release
```

### Subcommands

**`scrape`** — Fetch all coming-soon locations and update the DB.
```bash
cargo run -- scrape                          # Browser mode (default, handles Akamai)
cargo run -- scrape --cookie "COOKIE_STRING" # Cookie mode (faster, requires valid session)
cargo run -- scrape --file locations.json    # File mode (offline, for dev/replay)
cargo run -- scrape --country DE             # Different country code (US returns worldwide)
cargo run -- scrape --show-browser           # Show Chrome window instead of headless
```

**`status`** — Print a summary of the last scrape run and current DB state.
```bash
cargo run -- status
```

**`retry-failed`** — Re-fetch details for chargers where the previous detail fetch failed.
```bash
cargo run -- retry-failed
cargo run -- retry-failed --cookie "COOKIE_STRING"
cargo run -- retry-failed --show-browser
```

**`host`** — Start the read-only HTTP API server.
```bash
cargo run -- host            # Default port 8080
cargo run -- host --port 3000
```

## Testing

```bash
cargo test --verbose
```

CI runs on GitHub Actions (`.github/workflows/rust.yml`) on push/PR to `main`: builds and runs tests.

## Project Structure

```
src/
  main.rs              # CLI definition and subcommand dispatch
  coming_soon.rs       # ComingSoonSupercharger type, SiteStatus enum
  db.rs                # Database layer: queries, migrations, stats
  loaders.rs           # Data loading: browser / cookie / file modes
  raw.rs               # Raw API deserialization types
  sync.rs              # Diff logic: compute_sync, SyncPlan
  supercharger.rs      # Open (live) supercharger type
  display.rs           # Terminal table rendering
  api/
    mod.rs             # Axum router setup, error handling
    superchargers.rs   # Supercharger API endpoints
    scrape_runs.rs     # Scrape history endpoints

migrations/
  20260327000000_init.sql           # Full schema: tables, enums, indexes

docs/
  API.md               # HTTP API reference with response examples
  plans/               # Plan markdown files go here
```

## Architecture Notes

### Data Loading Modes
1. **Browser (default):** Launches headless Chrome via CDP, executes `fetch()` in-browser to preserve TLS fingerprint and cookies needed to bypass Akamai Bot Manager. Includes a ~6s delay for cookie generation.
2. **Cookie:** Uses a pre-obtained session cookie string with Reqwest directly — faster but requires manual auth.
3. **File:** Reads a local JSON dump — for offline development or replaying a previous response.

### Identifiers

Each coming-soon supercharger is identified by its Tesla location URL slug
(e.g. `"11255"` from `https://www.tesla.com/findus?location=11255`). This value is
stable across scrapes and is stored as `id` — the primary key — throughout the system.
Tesla's internal UUID field is intentionally ignored: it changes arbitrarily for the
same physical location and is therefore unreliable as an identifier.

### Database Schema
Three tables:
- `scrape_runs` — execution history (timestamp, country, counts, run type)
- `coming_soon_superchargers` — charger records (`id` = location slug, status, coordinates, fetch flags)
- `status_changes` — audit log of every status transition; `supercharger_id` FK references `coming_soon_superchargers.id`

All upserts and status changes are committed in a single transaction for atomicity.

### API
Read-only REST API, CORS-enabled, JSON responses. See `docs/API.md` for endpoint reference.

## Code Style

Standard Rust conventions. No custom `rustfmt.toml` or `.clippy.toml` — defaults apply. Use `cargo fmt` and `cargo clippy` before committing.

## Plans

Place all plan markdown files under `docs/plans/`.
