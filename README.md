# tesla-superchargers (soonerchargers.com)

Scrapes Tesla's internal Find Us API to track **coming-soon Supercharger locations** and persists them to Postgres so you can monitor status changes over time (e.g. when a site moves from *In Development* to *Under Construction*).

---

## How it works

Tesla's `findus` API returns 21k+ locations worldwide when queried with `?country=US`. This tool filters for `coming_soon_supercharger` entries, fetches per-location status details, and on each run:

- Fully upserts new or changed chargers (title, coordinates, status, raw status value)
- Touches `last_scraped_at` for chargers that haven't changed — so you always know the last time each site was confirmed present
- Records every status transition (`IN_DEVELOPMENT → UNDER_CONSTRUCTION` etc.) with the old and new value
- Marks chargers that disappear from the feed as inactive (`is_active = false`); `last_scraped_at` tells you when they were last seen
- Tracks chargers where the details fetch failed and lets you retry them without re-downloading the full location list

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

| Variable        | Required | Description |
|-----------------|----------|-------------|
| `DATABASE_URL`  | **Yes**  | Postgres connection string. Format: `postgres://user:password@host:5432/dbname` |

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

The tool uses subcommands: `scrape`, `status`, `retry-failed`, and `host`.

### `scrape` — fetch and persist all locations

```sh
# Launches Chrome headlessly, handles Akamai automatically
cargo run -- scrape

# Show the browser window while fetching (useful for debugging Akamai blocks)
cargo run -- scrape --show-browser

# Scrape a different country
cargo run -- scrape --country DE
```

#### `scrape` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--country <CODE>` | `US` | Country code passed to the API. `US` returns worldwide data. |
| `--show-browser` | off | Show the Chrome window instead of running headless. |

### `status` — show a summary of the last run and current DB state

```sh
cargo run -- status
```

Prints the last scrape run (timestamp, count, failures, status changes) and a breakdown of active chargers by status. If any chargers have failed detail fetches, it will indicate how many and suggest running `retry-failed`.

### `retry-failed` — re-fetch details for failed chargers

If some charger detail fetches failed during a `scrape`, those chargers are flagged in the DB. Use this command to retry only those, without re-downloading the full location list.

```sh
cargo run -- retry-failed

# Show browser window
cargo run -- retry-failed --show-browser
```

#### `retry-failed` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--show-browser` | off | Show the Chrome window instead of running headless. |

### `host` — start the HTTP API server

Starts a read-only HTTP API server that exposes the scraped data over JSON endpoints.

```sh
# Start on the default port (3000)
cargo run -- host

# Start on a custom port
cargo run -- host --port 8080
```

#### `host` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port <PORT>` | `3000` | Port to listen on. |

#### API endpoints

All endpoints are read-only and return JSON.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/superchargers/soon` | List all active coming-soon superchargers. |
| `GET` | `/superchargers/soon/stats` | Counts by status and timestamp of the last scrape. |
| `GET` | `/superchargers/soon/recent-changes` | Recent status transitions (e.g. `IN_DEVELOPMENT → UNDER_CONSTRUCTION`). |
| `GET` | `/superchargers/soon/recent-additions` | Superchargers first seen in recent scrapes. |
| `GET` | `/superchargers/soon/:uuid` | Detail for a single supercharger, including full status history. |
| `GET` | `/scrape-runs` | List recent scrape runs. |

##### Query parameters

**`GET /superchargers/soon`**

| Param | Default | Description |
|-------|---------|-------------|
| `status` | — | Filter by status: `IN_DEVELOPMENT`, `UNDER_CONSTRUCTION`, or `UNKNOWN`. |
| `limit` | `200` | Number of results (max 1000). |
| `offset` | `0` | Pagination offset. |

**`GET /superchargers/soon/recent-changes`** and **`GET /superchargers/soon/recent-additions`**

| Param | Default | Description |
|-------|---------|-------------|
| `limit` | `20` | Number of results (max 100). |
| `offset` | `0` | Pagination offset. |

**`GET /scrape-runs`**

| Param | Default | Description |
|-------|---------|-------------|
| `limit` | `10` | Number of results (max 50). |

---

## Database schema

Three tables are created automatically on first run:

**`scrape_runs`** — one row per execution, records the country, timestamp, run type (`full` or `retry`), coming-soon count, detail failure count, and status changes count.

**`coming_soon_superchargers`** — one row per unique charger (keyed by UUID). Tracks current status, coordinates, slug, when it was first seen (`first_seen_at`), when it was last confirmed present (`last_scraped_at`), whether it is still appearing in the feed (`is_active`), and whether the last details fetch failed (`details_fetch_failed`).

**`status_changes`** — append-only audit log. One row per status event: `old_status = NULL` means first sighting; a non-null `old_status` means a transition was detected. Linked to the scrape run that observed the change.

Status values: `IN_DEVELOPMENT`, `UNDER_CONSTRUCTION`, `UNKNOWN`.

---

## Notes

- The `?country=US` param is not a geographic filter — it switches the API into "full dataset" mode, returning all location types worldwide.
- The `filters=` param visible in the browser URL is UI-only and has no effect on the API response.
- Planned supercharger objects are minimal (no stall count, no open date). Status details are fetched separately from the location details endpoint.
- A charger disappearing from the feed does not necessarily mean it opened — it could also have been cancelled or removed. Check `last_scraped_at` to see when it was last confirmed present.
- Detail fetches that fail (network error, timeout, Akamai block) preserve the charger's existing status in the DB and set `details_fetch_failed = true`. Run `retry-failed` to resolve them without repeating the full scrape.
