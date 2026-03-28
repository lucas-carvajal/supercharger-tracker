# API Endpoints Plan for Supercharger Tracker Frontend

## Goal

Build a read-only HTTP API on top of the existing Postgres database so a frontend can display:
- All coming-soon superchargers, filterable by status
- Status distribution (in development / under construction / unknown)
- Status change history for individual sites
- Recent scrape run metadata (freshness indicator)

---

## Proposed Endpoints

### 1. `GET /api/coming-soon`

Returns all currently active coming-soon superchargers.

**Query parameters:**

| Param | Type | Description |
|-------|------|-------------|
| `status` | `in_development \| under_construction \| unknown` | Filter by status |
| `limit` | integer | Max results (default 200, max 1000) |
| `offset` | integer | Pagination offset (default 0) |

**Response body:**

```json
{
  "total": 412,
  "items": [
    {
      "uuid": "11399610",
      "title": "Highbridge, United Kingdom",
      "latitude": 51.22962,
      "longitude": -2.959685,
      "status": "in_development",
      "raw_status_value": "In Development",
      "location_url_slug": "11255",
      "tesla_url": "https://www.tesla.com/findus?location=11255",
      "first_seen_at": "2026-01-15T08:00:00Z",
      "last_scraped_at": "2026-03-27T06:00:00Z"
    }
  ]
}
```

**Why:** This is the primary data feed for the map/list view. Status filtering enables tab-based UI navigation. Pagination prevents large payloads.

---

### 2. `GET /api/coming-soon/stats`

Returns aggregate counts grouped by status.

**Response body:**

```json
{
  "total_active": 412,
  "by_status": {
    "in_development": 180,
    "under_construction": 195,
    "unknown": 37
  },
  "as_of": "2026-03-27T06:00:00Z"
}
```

`as_of` is the `scraped_at` timestamp of the latest scrape run.

**Why:** Drives summary cards / counters in the UI header without needing the full list.

---

### 3. `GET /api/coming-soon/:uuid`

Returns a single supercharger with its full status change history.

**Response body:**

```json
{
  "uuid": "11399610",
  "title": "Highbridge, United Kingdom",
  "latitude": 51.22962,
  "longitude": -2.959685,
  "status": "under_construction",
  "raw_status_value": "Under Construction",
  "location_url_slug": "11255",
  "tesla_url": "https://www.tesla.com/findus?location=11255",
  "first_seen_at": "2026-01-15T08:00:00Z",
  "last_scraped_at": "2026-03-27T06:00:00Z",
  "is_active": true,
  "status_history": [
    {
      "old_status": null,
      "new_status": "in_development",
      "changed_at": "2026-01-15T08:00:00Z"
    },
    {
      "old_status": "in_development",
      "new_status": "under_construction",
      "changed_at": "2026-02-20T06:00:00Z"
    }
  ]
}
```

**Why:** Powers a detail/sidebar view when a user clicks a map pin or list row. Status history shows progression toward opening.

---

### 4. `GET /api/coming-soon/recent-changes`

Returns the most recent status changes across all superchargers, newest first.

**Query parameters:**

| Param | Type | Description |
|-------|------|-------------|
| `limit` | integer | Max results (default 20, max 100) |
| `offset` | integer | Pagination offset (default 0) |

**Response body:**

```json
{
  "total": 54,
  "items": [
    {
      "uuid": "11399610",
      "title": "Highbridge, United Kingdom",
      "old_status": "in_development",
      "new_status": "under_construction",
      "changed_at": "2026-02-20T06:00:00Z"
    }
  ]
}
```

**Why:** A "recently updated" feed lets users see which sites have progressed. Particularly interesting is spotting the `in_development → under_construction` transition, which signals a site is closer to opening.

---

### 5. `GET /api/scrape-runs`

Returns recent scrape run metadata.

**Query parameters:**

| Param | Type | Description |
|-------|------|-------------|
| `limit` | integer | Max results (default 10, max 50) |

**Response body:**

```json
{
  "items": [
    {
      "id": 42,
      "country": "US",
      "scraped_at": "2026-03-27T06:00:00Z",
      "total_count": 412
    }
  ]
}
```

**Why:** Lets the UI show a "last updated" timestamp and a simple history of how the total count has changed over time (growth chart).

---

## Implementation Plan

### 1. Add web server dependency

Add `axum` and `tokio` (already present) to `Cargo.toml`. Also add `tower-http` for CORS middleware (required for browser-based frontend).

```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["cors"] }
```

### 2. New module: `src/api.rs`

Create a new module that owns the Axum router and all handler functions.

```
src/
├── api.rs          ← new: router + all handlers
├── api/
│   ├── mod.rs      ← re-exports
│   ├── coming_soon.rs  ← handlers for /api/coming-soon/*
│   └── scrape_runs.rs  ← handler for /api/scrape-runs
```

Or a single flat `src/api.rs` if the handlers stay small.

### 3. New DB query functions in `src/db.rs`

Add the following read-only query functions:

| Function | SQL | Used by |
|----------|-----|---------|
| `list_coming_soon(status_filter, limit, offset)` | `SELECT … FROM coming_soon_superchargers WHERE is_active = true [AND status = $1] ORDER BY title LIMIT $2 OFFSET $3` | endpoint 1 |
| `count_coming_soon_by_status()` | `SELECT status, COUNT(*) … GROUP BY status` | endpoint 2 |
| `get_coming_soon(uuid)` | `SELECT … FROM coming_soon_superchargers WHERE uuid = $1` | endpoint 3 |
| `get_status_history(uuid)` | `SELECT … FROM status_changes WHERE supercharger_uuid = $1 ORDER BY changed_at` | endpoint 3 |
| `list_recent_changes(limit, offset)` | `SELECT sc.*, cs.title FROM status_changes sc JOIN coming_soon_superchargers cs … ORDER BY changed_at DESC` | endpoint 4 |
| `list_scrape_runs(limit)` | `SELECT … FROM scrape_runs ORDER BY scraped_at DESC LIMIT $1` | endpoint 5 |
| `latest_scrape_run()` | `SELECT … FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1` | endpoint 2 |

### 4. Update `src/main.rs`

Add a `serve` subcommand (or a `--serve` flag) that starts the Axum HTTP server instead of running the scraper:

```
cargo run -- serve --port 3000
```

The server shares the same `PgPool` established by `db::connect()`. No scraping happens while serving — the two modes are independent.

### 5. Response serialization

All handlers return `axum::Json<T>` where `T` derives `serde::Serialize`. Add `chrono` feature to serde for timestamp serialization (already a transitive dependency via sqlx).

### 6. CORS

Configure `tower_http::cors::CorsLayer` to allow the frontend origin during development (`http://localhost:5173` or `*` in dev, restricted in production).

### 7. Error handling

Define a shared `ApiError` type that implements `IntoResponse`, returning JSON error bodies:

```json
{ "error": "supercharger not found" }
```

---

## What is NOT in scope

- Authentication / API keys (read-only public data)
- Write endpoints (scraping is still triggered via CLI)
- WebSocket / real-time push (polling the scrape-runs endpoint is sufficient)
- Caching layer (Postgres is fast enough for read queries at this scale)

---

## Summary

| # | Endpoint | Purpose |
|---|----------|---------|
| 1 | `GET /api/coming-soon` | Map/list feed, filterable by status |
| 2 | `GET /api/coming-soon/stats` | Summary counts for UI header |
| 3 | `GET /api/coming-soon/:uuid` | Detail view with status history |
| 4 | `GET /api/coming-soon/recent-changes` | "Recently updated" feed |
| 5 | `GET /api/scrape-runs` | Data freshness / count history |
