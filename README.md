# tesla-superchargers

Scrapes Tesla's internal Find Us API to track **coming-soon Supercharger locations** and persists them to Postgres so you can monitor status changes over time (e.g. when a site moves from *In Development* to *Under Construction*).

---

## How it works

Tesla's `findus` API returns 21k+ locations worldwide when queried with `?country=US`. This tool filters for `coming_soon_supercharger` entries, fetches per-location status details, and saves everything to Postgres on each run:

- Upserts all coming-soon locations
- Records status transitions (`IN_DEVELOPMENT → UNDER_CONSTRUCTION` etc.)
- Marks locations that disappeared from the feed as inactive (`is_active = false`)

---

## Setup

### 1. Prerequisites

- Rust (stable) — [rustup.rs](https://rustup.rs)
- Postgres
- Chrome or Chromium — used headlessly to bypass Akamai bot protection when fetching live data
  - macOS: install [Google Chrome](https://www.google.com/chrome/)
  - Linux/server: `apt install chromium-browser`

### 2. Environment variables

Copy `.env.example` to `.env` and fill in your values:

```sh
cp .env.example .env
```

| Variable       | Required | Description |
|----------------|----------|-------------|
| `DATABASE_URL` | **Yes**  | Postgres connection string. Format: `postgres://user:password@host:5432/dbname` |

### 3. Database

Spin up Postgres locally with Docker:

```sh
docker run -d --name supercharger-db \
  -e POSTGRES_DB=supercharger-db \
  -e POSTGRES_PASSWORD=pass \
  -p 5432:5432 postgres:16
```

Migrations run automatically on startup — no manual steps needed.

### 4. Build

```sh
cargo build --release
```

---

## Usage

```sh
# Scrape and persist (DATABASE_URL set in .env)
cargo run

# Use a local JSON dump instead of fetching live
cargo run -- --file locations.json

# Show the browser window while fetching (useful for debugging)
cargo run -- --show-browser

# Also print the table of open superchargers
cargo run -- --show-open
```

### All flags

| Flag | Default | Description |
|------|---------|-------------|
| `--file <PATH>` | — | Read from a local JSON file instead of fetching live. |
| `--country <CODE>` | `US` | Country code passed to the API. `US` returns worldwide data. |
| `--show-browser` | off | Show the Chrome window instead of running headless. Useful for debugging Akamai blocks. |
| `--show-open` | off | Also print the table of open superchargers. |

---

## Notes

- The `?country=US` param is not a geographic filter — it switches the API into "full dataset" mode, returning all location types worldwide.
- The `filters=` param visible in the browser URL is UI-only and has no effect on the API response.
- Planned supercharger objects are minimal (no stall count, no open date). Status details are fetched separately from the location details endpoint.
