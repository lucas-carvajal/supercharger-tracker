# API Reference

The supercharger-tracker HTTP API exposes read-only data scraped from Tesla's coming-soon supercharger feed.

**Base URL:** `http://localhost:3000` (port configurable via `--port`)
**Auth:** Read endpoints are unauthenticated. `POST /scrapes/import` requires an `X-Import-Token` header (see below).
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

| Value | Meaning |
|---|---|
| `IN_DEVELOPMENT` | Site is in development |
| `UNDER_CONSTRUCTION` | Site is under construction |
| `UNKNOWN` | Status could not be determined |
| `REMOVED` | Charger disappeared from the Tesla feed and was not found to have opened |

---

## Endpoints

### `GET /superchargers/soon`

List all active coming-soon superchargers.

**Query parameters**

| Param | Type | Default | Max | Description |
|---|---|---|---|---|
| `status` | string | — | — | Filter by status (case-insensitive): `IN_DEVELOPMENT`, `UNDER_CONSTRUCTION`, `UNKNOWN` |
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
      "status": "IN_DEVELOPMENT",
      "raw_status_value": "In Development",
      "tesla_url": "https://www.tesla.com/findus?location=11255",
      "first_seen_at": "2026-03-15T10:30:00Z",
      "last_scraped_at": "2026-03-31T08:45:00Z",
      "details_fetch_failed": false
    }
  ]
}
```

`city` and `region` are `null` for entries where Tesla's title could not be parsed (e.g. `"locations"`, or titles with no comma).

---

### `GET /superchargers/soon/stats`

Aggregate counts by status, plus the timestamp of the most recent scrape.

**Response**

```json
{
  "total_active": 806,
  "by_status": {
    "IN_DEVELOPMENT": 450,
    "UNDER_CONSTRUCTION": 320,
    "UNKNOWN": 36
  },
  "as_of": "2026-03-31T08:45:00Z"
}
```

`as_of` is `null` if no scrape runs exist yet.

---

### `GET /superchargers/soon/recent-changes`

Recent status transitions across all superchargers, ordered by most recent first.

**Query parameters**

| Param | Type | Default | Max |
|---|---|---|---|
| `limit` | integer | 20 | 100 |
| `offset` | integer | 0 | — |

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
      "old_status": "IN_DEVELOPMENT",
      "new_status": "UNDER_CONSTRUCTION",
      "changed_at": "2026-03-28T14:15:00Z"
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
      "status": "IN_DEVELOPMENT",
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
  "status": "UNDER_CONSTRUCTION",
  "raw_status_value": "Under Construction",
  "tesla_url": "https://www.tesla.com/findus?location=11255",
  "first_seen_at": "2026-03-15T10:30:00Z",
  "last_scraped_at": "2026-03-31T08:45:00Z",
  "details_fetch_failed": false,
  "status_history": [
    {
      "old_status": null,
      "new_status": "IN_DEVELOPMENT",
      "changed_at": "2026-03-15T10:30:00Z"
    },
    {
      "old_status": "IN_DEVELOPMENT",
      "new_status": "UNDER_CONSTRUCTION",
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

### `POST /scrapes/import`

Apply a diff or snapshot export file generated by `export-diff` or `export-snapshot`.
Used to transfer scrape results from the local (VPN-gated) machine to prod.

**Auth:** Requires `X-Import-Token: <secret>` header matching the `IMPORT_TOKEN` env var on the server. Returns `401` if the token is wrong and `503` if `IMPORT_TOKEN` is not configured.

**Query parameters**

| Param | Type | Default | Description |
|---|---|---|---|
| `force` | bool | false | Bypass the ordering check (for gap recovery) |

**Request body:** JSON — a `ScrapeExport` object as produced by `export-diff` or `export-snapshot`.

**Example**

```bash
curl -X POST https://prod/scrapes/import \
  -H "X-Import-Token: your-secret" \
  -H "Content-Type: application/json" \
  -d @scrape_export_42.json
```

**Response**

```json
{ "status": "applied", "run_id": 42, "changed": 15, "opened": 1, "removed": 2 }
```

| `status` | HTTP | Meaning |
|---|---|---|
| `applied` | 200 | Diff was applied successfully |
| `duplicate` | 200 | This run_id was already imported — no-op |
| `out_of_order` | 409 | `run_id` is not `MAX(id) + 1`; a prior export may be missing |
| `snapshot_applied` | 200 | Snapshot was applied; all four tables replaced |

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
