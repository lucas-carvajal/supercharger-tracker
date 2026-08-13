# API Reference

The supercharger-tracker HTTP API exposes read-only data scraped from Tesla's coming-soon supercharger feed.

**Base URL:** `http://localhost:3000` (port configurable via `--port`)
**Auth:** Read endpoints are unauthenticated. `POST /admin/import/scrapes` requires an `X-Admin-Internal-Secret` header for trusted internal Next.js -> Rust calls (see below).
**CORS:** All origins allowed
**Timestamps:** UTC, ISO 8601 (e.g. `2026-03-31T08:45:00Z`)

---

## Identifiers

Each supercharger is identified by an `id` field, which is the Tesla location URL slug
(e.g. `"11255"` from `https://www.tesla.com/findus?location=11255`). This value is stable
across scrapes and used as the primary key throughout the system. Tesla's internal UUID
field is intentionally not exposed — it changes arbitrarily for the same location.

---

## Status values

Build pipeline order: `PRELIMINARY → DESIGN → CONSTRUCTION → (OPENED)`.

| Value | Meaning |
|---|---|
| `PRELIMINARY` | Earliest build stage (site picked / voted; planning not yet underway) |
| `DESIGN` | Planning underway |
| `CONSTRUCTION` | Actively being built |
| `UNKNOWN` | Status could not be determined |
| `OPENED` | Charger was confirmed open via the Tesla API (appears only in `status_changes` history; the charger record moves to `opened_superchargers`) |
| `REMOVED` | Charger disappeared from the Tesla feed and was not found to have opened |

Status is derived from Tesla's `project_status` when available, falling back to the
customer-facing label (`raw_status_value`). The raw Tesla `project_status` string is also
exposed as `raw_project_status` (title case, informational).

---

## Endpoints

### `GET /health`

Simple health check.

**Response**

```json
{ "status": "ok" }
```

---

### `GET /superchargers/soon`

List all active coming-soon superchargers.

**Query parameters**

| Param | Type | Default | Max | Description |
|---|---|---|---|---|
| `status` | string | — | — | Filter by status (case-insensitive): `PRELIMINARY`, `DESIGN`, `CONSTRUCTION`, `UNKNOWN` |
| `region` | string | — | — | Filter by region (see below) |
| `limit` | integer | 200 | 1000 | Number of results |
| `offset` | integer | 0 | — | Pagination offset |

**`?region=` values**

| Input | Matches |
|---|---|
| `US` | All US states + DC |
| `CA`, `TX`, `NY`, … (any US state/DC code) | That state only |
| `AU` or `Australia` | All Australian states/territories |
| `NSW`, `VIC`, `QLD`, `SA`, `WA`, `TAS`, `NT`, `ACT` | That AU territory |
| `Canada` | All Canadian provinces/territories |
| `BC`, `ON`, `AB`, `SK`, `MB`, `QC`, `NB`, `NS`, `PE`, `NL`, `YT`, `NU` | That Canadian province |
| `Mexico` | All Mexican state variants |
| `BCS`, `COAH`, … (Mexican state codes) | That Mexican state |
| `United Kingdom` or `UK` | Both `"United Kingdom"` and `"UK"` DB entries |
| `Turkey`, `Turkiye`, or `Türkiye` | Both Turkish spelling variants |
| `UAE` or `United Arab Emirates` | All UAE variants |
| `New Zealand` or `NZ` | Both NZ spelling variants |
| `Germany`, `France`, `Spain`, `Norway`, `Sweden`, `Italy`, `Finland`, `Denmark`, `Hungary`, `Romania`, `Czech Republic`, `Iceland`, `Ireland`, `Portugal`, `Croatia`, `Slovenia`, `Slovakia`, `Switzerland`, `Austria`, `Netherlands`, `Poland`, `Latvia`, `Morocco`, `Taiwan`, `Thailand`, `Japan`, `South Korea`, `Chile`, `Colombia`, `Israel`, `Saudi Arabia` | That country |

Matching is case-insensitive. Unknown values return `400 Bad Request`.

**Note:** `?region=NT` matches both Australian Northern Territory and Canadian Northwest Territories, since Tesla uses the same `NT` code for both.

**Response**

```json
{
  "total": 42,
  "items": [
    {
      "id": "11255",
      "title": "Highbridge, United Kingdom",
      "city": "Highbridge",
      "region": "United Kingdom",
      "latitude": 51.22962,
      "longitude": -2.959685,
      "status": "DESIGN",
      "raw_status_value": "In Development",
      "raw_project_status": "Design",
      "num_charger_stalls": 8,
      "charging_accessibility": "Tesla Only",
      "tesla_url": "https://www.tesla.com/findus?location=11255",
      "first_seen_at": "2026-03-15T10:30:00Z",
      "last_scraped_at": "2026-03-31T08:45:00Z",
      "details_fetch_failed": false
    }
  ]
}
```

`city` and `region` are `null` for entries where Tesla's title could not be parsed (e.g. `"locations"`, or titles with no comma).

`num_charger_stalls: 0` means the stall count is unknown / not yet published by Tesla — treat
as "—", not "0 stalls". Structured address fields are stored in the DB but not exposed via the API.

---

### `GET /superchargers/soon/map`

All active coming-soon superchargers as lightweight map markers. Returns a flat JSON array (not paginated).

**Response**

```json
[
  {
    "id": "11255",
    "title": "Highbridge, United Kingdom",
    "latitude": 51.22962,
    "longitude": -2.959685,
    "status": "DESIGN",
    "num_charger_stalls": 8
  }
]
```

`num_charger_stalls` uses the same semantics as list/detail: a positive integer when known;
`0` means unknown / not yet published by Tesla (treat as “don’t show”, not “0 stalls”).
The field is always present (never omitted or null).

---

### `GET /superchargers/soon/stats`

Aggregate counts by status, plus the timestamp of the most recent scrape.

**Response**

```json
{
  "total_active": 806,
  "by_status": {
    "PRELIMINARY": 180,
    "DESIGN": 270,
    "CONSTRUCTION": 320,
    "UNKNOWN": 36
  },
  "as_of": "2026-03-31T08:45:00Z"
}
```

`as_of` is `null` if no scrape runs exist yet.

---

### `GET /superchargers/soon/recent-changes`

Recent status transitions across all superchargers, ordered by most recent first.
Includes `OPENED` and `REMOVED` transitions, excludes first-seen rows (`old_status = null`),
and excludes transitions where `new_status = UNKNOWN`.

**Query parameters**

| Param | Type | Default | Max |
|---|---|---|---|
| `limit` | integer | 20 | 100 |
| `offset` | integer | 0 | — |

Ordering is deterministic: `changed_at DESC`, then `id DESC` for tie-breaking.

**Response**

```json
{
  "total": 45,
  "items": [
    {
      "id": "11255",
      "title": "Highbridge, United Kingdom",
      "city": "Highbridge",
      "region": "United Kingdom",
      "old_status": "DESIGN",
      "new_status": "CONSTRUCTION",
      "changed_at": "2026-03-28T14:15:00Z"
    }
  ]
}
```

---

### `GET /superchargers/soon/recent-updates`

Combined activity feed of first-seen rows and status transitions, ordered by most recent first.
Includes first-seen events (`old_status = null`) and `OPENED` transitions.
Excludes `new_status = REMOVED` and `new_status = UNKNOWN`.

**Query parameters**

| Param | Type | Default | Max |
|---|---|---|---|
| `limit` | integer | 20 | 100 |
| `offset` | integer | 0 | — |

Ordering is deterministic: `changed_at DESC`, then `id DESC` for tie-breaking.

**Response**

```json
{
  "total": 57,
  "items": [
    {
      "id": "11255",
      "title": "Highbridge, United Kingdom",
      "city": "Highbridge",
      "region": "United Kingdom",
      "old_status": "DESIGN",
      "new_status": "CONSTRUCTION",
      "changed_at": "2026-03-28T14:15:00Z"
    },
    {
      "id": "12001",
      "title": "Austin, TX",
      "city": "Austin",
      "region": "TX",
      "old_status": null,
      "new_status": "PRELIMINARY",
      "changed_at": "2026-03-27T10:00:00Z"
    }
  ]
}
```

---

### `GET /superchargers/soon/recent-additions`

Superchargers first seen in recent scrapes, ordered by most recently added first.

**Query parameters**

| Param | Type | Default | Max |
|---|---|---|---|
| `limit` | integer | 20 | 100 |
| `offset` | integer | 0 | — |

**Response**

```json
{
  "total": 12,
  "items": [
    {
      "id": "11255",
      "title": "Highbridge, United Kingdom",
      "city": "Highbridge",
      "region": "United Kingdom",
      "latitude": 51.22962,
      "longitude": -2.959685,
      "status": "PRELIMINARY",
      "raw_status_value": "In Development",
      "tesla_url": "https://www.tesla.com/findus?location=11255",
      "first_seen_at": "2026-03-29T10:30:00Z"
    }
  ]
}
```

---

### `GET /superchargers/soon/:id`

Single supercharger with full status history.

**Path parameters**

| Param | Description |
|---|---|
| `id` | Supercharger ID (Tesla location URL slug, e.g. `"11255"`) |

**Response**

```json
{
  "id": "11255",
  "title": "Highbridge, United Kingdom",
  "city": "Highbridge",
  "region": "United Kingdom",
  "latitude": 51.22962,
  "longitude": -2.959685,
  "status": "CONSTRUCTION",
  "raw_status_value": "Under Construction",
  "raw_project_status": "Construction",
  "num_charger_stalls": 8,
  "charging_accessibility": "Tesla Only",
  "tesla_url": "https://www.tesla.com/findus?location=11255",
  "first_seen_at": "2026-03-15T10:30:00Z",
  "last_scraped_at": "2026-03-31T08:45:00Z",
  "details_fetch_failed": false,
  "status_history": [
    {
      "old_status": null,
      "new_status": "PRELIMINARY",
      "changed_at": "2026-03-15T10:30:00Z"
    },
    {
      "old_status": "PRELIMINARY",
      "new_status": "DESIGN",
      "changed_at": "2026-03-20T09:00:00Z"
    },
    {
      "old_status": "DESIGN",
      "new_status": "CONSTRUCTION",
      "changed_at": "2026-03-28T14:15:00Z"
    }
  ]
}
```

`old_status` is `null` for the first-seen entry. Chargers with `status = "REMOVED"` disappeared
from the Tesla feed and were confirmed not to have opened. Opened chargers are removed from this
table entirely and can be found in the `opened_superchargers` table.

**Errors:** `404` if the ID is not found.

---

### `POST /admin/import/scrapes`

Apply a diff or snapshot export file generated by `export-diff` or `export-snapshot`.
Used to transfer scrape results from the local (VPN-gated) machine to prod.

**Auth:** Requires `X-Admin-Internal-Secret: <secret>` header matching the `RUST_INTERNAL_IMPORT_SECRET` env var on the server. Returns `401` if the secret is wrong and `503` if `RUST_INTERNAL_IMPORT_SECRET` is not configured.

**Query parameters**

| Param | Type | Default | Description |
|---|---|---|---|
| `force` | bool | false | Bypass the ordering check (for gap recovery) |

**Request body:** JSON — a `ScrapeExport` object as produced by `export-diff` or `export-snapshot`. Opened chargers in snapshot/diff payloads may include `installed_full_power_kw` (integer kW, or omitted when unknown). This field is stored on graduation but is **not** exposed by the public read API.

**Example**

```bash
curl -X POST https://prod/admin/import/scrapes \
  -H "X-Admin-Internal-Secret: your-secret" \
  -H "Content-Type: application/json" \
  -d @scrape_export_42.json
```

**Responses**

| `status` | HTTP | Meaning |
|---|---|---|
| `applied` | 200 | Diff was applied successfully |
| `duplicate` | 200 | This run_id was already imported — no-op |
| `out_of_order` | 409 | `run_id` is not `MAX(id) + 1`; a prior export may be missing |
| `snapshot_applied` | 200 | Snapshot was applied; all four tables replaced |

```json
{ "status": "applied", "run_id": 42, "changed": 15, "opened": 1, "removed": 2 }
```

```json
{ "status": "duplicate", "run_id": 42 }
```

```json
{ "status": "out_of_order", "expected": 43, "got": 45 }
```

```json
{ "status": "snapshot_applied", "source_run_id": 1, "scrape_runs": 42, "chargers": 806, "opened": 25 }
```

> **Fresh prod instance:** always apply a snapshot before applying diffs. On an empty DB, `MAX(id)` is 0 so the ordering check expects `run_id = 1`, which will never match a real local run. Use `export-snapshot` on local and import it first.

---

### `GET /admin/import/current-version`

Return the current import version on this instance and the next version expected for an
incremental diff. `current_version` is the maximum `scrape_runs.id` currently stored in
the database, or `0` when no scrape runs exist.

**Auth:** Requires `X-Admin-Internal-Secret: <secret>` header matching the `RUST_INTERNAL_IMPORT_SECRET` env var on the server. Returns `401` if the secret is wrong and `503` if `RUST_INTERNAL_IMPORT_SECRET` is not configured.

**Example**

```bash
curl https://prod/admin/import/current-version \
  -H "X-Admin-Internal-Secret: your-secret"
```

**Response**

```json
{ "current_version": 42, "next_expected_version": 43 }
```

---

### `GET /scrape-runs`

Recent scrape run records, ordered by most recent first.

**Query parameters**

| Param | Type | Default | Max |
|---|---|---|---|
| `limit` | integer | 10 | 50 |

**Response**

```json
{
  "items": [
    {
      "id": 42,
      "country": "US",
      "scraped_at": "2026-03-31T08:45:00Z",
      "total_count": 806
    }
  ]
}
```

---

## Errors

All errors return JSON with an `error` field.

```json
{
  "error": "supercharger not found"
}
```

| Status | Cause |
|---|---|
| `400` | Invalid query parameter (e.g. unrecognised `status` value) |
| `404` | Resource not found |
| `500` | Internal server error |

---

## Pagination

Endpoints that support pagination use `limit` and `offset` query parameters. Responses
include a `total` field with the full count of matching records regardless of the current page.

```
GET /superchargers/soon?limit=50&offset=100
```
