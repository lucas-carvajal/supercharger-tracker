# API Endpoints Plan for Supercharger Tracker Frontend

## Goal

Build a read-only HTTP API on top of the existing Postgres database so a frontend can display:
- All coming-soon superchargers, filterable by status
- Status distribution (in development / under construction / unknown)
- Status change history for individual sites
- Recent scrape run metadata (freshness indicator)

---

## Proposed Endpoints

### 1. `GET /superchargers/soon`

Returns all currently active coming-soon superchargers.

**Query parameters:**

| Param | Type | Description |
|-------|------|-------------|
| `status` | `IN_DEVELOPMENT \| UNDER_CONSTRUCTION \| UNKNOWN` | Filter by status (case-insensitive) |
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
      "status": "IN_DEVELOPMENT",
      "raw_status_value": "In Development",
      "location_url_slug": "11255",
      "tesla_url": "https://www.tesla.com/findus?location=11255",
      "first_seen_at": "2026-01-15T08:00:00Z",
      "last_scraped_at": "2026-03-27T06:00:00Z",
      "details_fetch_failed": false
    }
  ]
}
```

**Nullable fields:** `raw_status_value`, `location_url_slug`, and `tesla_url` (derived from `location_url_slug`) may be `null` for chargers where `details_fetch_failed = true`. The `details_fetch_failed` flag is included in the response so the frontend can handle these gracefully (e.g. disable the detail link).

**Why:** This is the primary data feed for the map/list view. Status filtering enables tab-based UI navigation. Pagination prevents large payloads.

**Implementation details:**

1. Parse and validate query params: clamp `limit` to 1–1000 (default 200), `offset` ≥ 0 (default 0), validate `status` case-insensitively against the enum if provided (uppercase before passing to SQL).
2. Run two queries against `coming_soon_superchargers`:
   - `SELECT COUNT(*) WHERE is_active = true [AND status = $1]` → `total`
   - `SELECT … WHERE is_active = true [AND status = $1] ORDER BY title LIMIT $2 OFFSET $3` → `items`
3. Status values are returned as-is from the DB (uppercase enum strings).

---

### 2. `GET /superchargers/soon/stats`

Returns aggregate counts grouped by status.

**Response body:**

```json
{
  "total_active": 412,
  "by_status": {
    "IN_DEVELOPMENT": 180,
    "UNDER_CONSTRUCTION": 195,
    "UNKNOWN": 37
  },
  "as_of": "2026-03-27T06:00:00Z"
}
```

`as_of` is the `scraped_at` timestamp of the most recent scrape run. Note: if scrape runs are recorded per country, this reflects the globally latest run, which may not correspond to the country of every charger shown.

**Why:** Drives summary cards / counters in the UI header without needing the full list.

**Implementation details:**

1. Run `SELECT status, COUNT(*) FROM coming_soon_superchargers WHERE is_active = true GROUP BY status`.
2. Build the `by_status` map from the results; explicitly set any missing status key to `0` so the response shape is always consistent.
3. Sum all counts to produce `total_active`.
4. Run `SELECT scraped_at FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1` to get `as_of`. If the table is empty, return `null`.

---

### 3. `GET /superchargers/soon/:uuid`

Returns a single supercharger with its full status change history. Returns both active and inactive (disappeared) chargers — `is_active` in the response indicates current state.

**Response body:**

```json
{
  "uuid": "11399610",
  "title": "Highbridge, United Kingdom",
  "latitude": 51.22962,
  "longitude": -2.959685,
  "status": "UNDER_CONSTRUCTION",
  "raw_status_value": "Under Construction",
  "location_url_slug": "11255",
  "tesla_url": "https://www.tesla.com/findus?location=11255",
  "first_seen_at": "2026-01-15T08:00:00Z",
  "last_scraped_at": "2026-03-27T06:00:00Z",
  "is_active": true,
  "details_fetch_failed": false,
  "status_history": [
    {
      "old_status": null,
      "new_status": "IN_DEVELOPMENT",
      "changed_at": "2026-01-15T08:00:00Z"
    },
    {
      "old_status": "IN_DEVELOPMENT",
      "new_status": "UNDER_CONSTRUCTION",
      "changed_at": "2026-02-20T06:00:00Z"
    }
  ]
}
```

**Why:** Powers a detail/sidebar view when a user clicks a map pin or list row. Status history shows progression toward opening.

**Implementation details:**

1. Query `SELECT … FROM coming_soon_superchargers WHERE uuid = $1` (no `is_active` filter — inactive chargers are still returned). Return `404` with `{ "error": "not found" }` if no row.
2. Query `SELECT old_status, new_status, changed_at FROM status_changes WHERE supercharger_uuid = $1 ORDER BY changed_at ASC`. May return zero rows.
3. Combine both results. `old_status` on the first history entry will be `null` (first observation).

---

### 4. `GET /superchargers/soon/recent-changes`

Returns the most recent status transitions across all superchargers, newest first. First appearances (where a charger was seen for the first time) are excluded — only actual status transitions are returned.

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
      "old_status": "IN_DEVELOPMENT",
      "new_status": "UNDER_CONSTRUCTION",
      "changed_at": "2026-02-20T06:00:00Z"
    }
  ]
}
```

**Why:** A "recently updated" feed lets users see which sites have progressed. Particularly interesting is spotting the `IN_DEVELOPMENT → UNDER_CONSTRUCTION` transition, which signals a site is closer to opening.

**Implementation details:**

1. Parse and validate query params: clamp `limit` to 1–100 (default 20), `offset` ≥ 0 (default 0).
2. Run two queries, both filtering `WHERE sc.old_status IS NOT NULL` to exclude first-seen inserts:
   - `SELECT COUNT(*) FROM status_changes WHERE old_status IS NOT NULL` → `total`
   - `SELECT sc.old_status, sc.new_status, sc.changed_at, cs.title, cs.uuid FROM status_changes sc JOIN coming_soon_superchargers cs ON cs.uuid = sc.supercharger_uuid WHERE sc.old_status IS NOT NULL ORDER BY sc.changed_at DESC LIMIT $1 OFFSET $2` → `items`

---

### 5. `GET /scrape-runs`

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

**Implementation details:**

1. Parse and validate query param: clamp `limit` to 1–50 (default 10).
2. Run `SELECT id, country, scraped_at, total_count FROM scrape_runs ORDER BY scraped_at DESC LIMIT $1`.
3. `total_count` is nullable in the schema — default to `0` in the response if `null`.

---

## Implementation Plan

### 1. Add web server dependency

Add `axum` and `tokio` (already present) to `Cargo.toml`. Also add `tower-http` for CORS middleware (required for browser-based frontend).

```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["cors"] }
```

### 2. New module: `src/api/`

Create a new `src/api/` module with the following files. `main.rs` calls `api::router()` to get the Axum router and binds it to the configured port.

```
src/
└── api/
    ├── mod.rs           ← exposes router() fn, shared ApiError type
    ├── superchargers.rs ← handlers for /superchargers/soon/*
    └── scrape_runs.rs   ← handler for /scrape-runs
```

**Route registration order:** Within `router()`, static routes must be registered before the dynamic `/:uuid` route to prevent Axum capturing them as path params:

```rust
Router::new()
    .route("/superchargers/soon/stats", get(stats_handler))
    .route("/superchargers/soon/recent-changes", get(recent_changes_handler))
    .route("/superchargers/soon/:uuid", get(detail_handler))
    .route("/superchargers/soon", get(list_handler))
    .route("/scrape-runs", get(scrape_runs_handler))
```

### 3. New DB query functions in `src/db.rs`

Add the following read-only query functions:

| Function | SQL | Used by |
|----------|-----|---------|
| `list_coming_soon(status_filter, limit, offset)` | `SELECT … FROM coming_soon_superchargers WHERE is_active = true [AND status = $1] ORDER BY title LIMIT $2 OFFSET $3` | endpoint 1 |
| `count_coming_soon_by_status()` | `SELECT status, COUNT(*) … GROUP BY status` | endpoint 2 |
| `get_coming_soon(uuid)` | `SELECT … FROM coming_soon_superchargers WHERE uuid = $1` | endpoint 3 |
| `get_status_history(uuid)` | `SELECT … FROM status_changes WHERE supercharger_uuid = $1 ORDER BY changed_at` | endpoint 3 |
| `list_recent_changes(limit, offset)` | `SELECT sc.*, cs.title FROM status_changes sc JOIN coming_soon_superchargers cs … WHERE sc.old_status IS NOT NULL ORDER BY changed_at DESC` | endpoint 4 |
| `list_scrape_runs(limit)` | `SELECT … FROM scrape_runs ORDER BY scraped_at DESC LIMIT $1` | endpoint 5 |
| `latest_scrape_run()` | `SELECT … FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1` | endpoint 2 |

### 4. Update `src/main.rs`

Add `Host` as a fourth variant to the existing `Command` enum alongside `Scrape`, `RetryFailed`, and `Status`:

```rust
/// Start the HTTP API server.
Host {
    /// Port to listen on (default: 3000).
    #[arg(short, long, default_value = "3000")]
    port: u16,
},
```

Add a corresponding `run_host` handler that calls `api::router()`, wraps it with CORS middleware, and starts `axum::serve`:

```
cargo run -- host --port 3000
```

The four CLI commands are fully independent — `host` only reads from the DB, no scraping happens while it is running.

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

## Index Analysis

Existing indexes in the migration:

```sql
CREATE INDEX ON status_changes (supercharger_uuid);
CREATE INDEX ON coming_soon_superchargers (status);
CREATE INDEX ON coming_soon_superchargers (is_active);
CREATE INDEX ON coming_soon_superchargers (details_fetch_failed) WHERE details_fetch_failed = TRUE;
```

| Endpoint | Query pattern | Existing coverage | Gap |
|---|---|---|---|
| `GET /superchargers/soon` | `WHERE is_active = true [AND status = $1] ORDER BY title` | `is_active` and `status` indexes exist | No composite `(is_active, status)` — Postgres will use one index then filter. Fine at current scale. |
| `GET /superchargers/soon/stats` | `WHERE is_active = true GROUP BY status` | `is_active` index covers the filter | Same as above — acceptable |
| `GET /superchargers/soon/:uuid` | PK lookup + `WHERE supercharger_uuid = $1 ORDER BY changed_at` | PK covers the first query; `supercharger_uuid` index covers the second | No index on `status_changes.changed_at` — sort happens in memory after filtering by uuid. Fine since a single charger has few history rows. |
| `GET /superchargers/soon/recent-changes` | `WHERE old_status IS NOT NULL ORDER BY changed_at DESC` | No index on `changed_at` | **Missing index** — full table scan + sort on every request. Needs `CREATE INDEX ON status_changes (changed_at DESC)`. |
| `GET /scrape-runs` | `ORDER BY scraped_at DESC LIMIT $1` | No index on `scraped_at` | Table will stay small (one row per scrape run), so a seq scan is fine. |

**Required new index** (add to a new migration):

```sql
CREATE INDEX ON status_changes (changed_at DESC);
```

This turns the recent-changes query from a full table scan into an index scan, which matters as the `status_changes` table grows with every scrape run.

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
| 1 | `GET /superchargers/soon` | Map/list feed, filterable by status |
| 2 | `GET /superchargers/soon/stats` | Summary counts for UI header |
| 3 | `GET /superchargers/soon/:uuid` | Detail view with status history |
| 4 | `GET /superchargers/soon/recent-changes` | "Recently updated" feed |
| 5 | `GET /scrape-runs` | Data freshness / count history |
