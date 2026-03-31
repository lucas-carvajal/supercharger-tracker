# API Reference

The supercharger-tracker HTTP API exposes read-only data scraped from Tesla's coming-soon supercharger feed.

**Base URL:** `http://localhost:3000` (port configurable via `--port`)
**Auth:** None
**CORS:** All origins allowed
**Timestamps:** UTC, ISO 8601 (e.g. `2026-03-31T08:45:00Z`)

---

## Status values

| Value | Meaning |
|---|---|
| `IN_DEVELOPMENT` | Site is in development |
| `UNDER_CONSTRUCTION` | Site is under construction |
| `UNKNOWN` | Status could not be determined |

---

## Endpoints

### `GET /superchargers/soon`

List all active coming-soon superchargers.

**Query parameters**

| Param | Type | Default | Max | Description |
|---|---|---|---|---|
| `status` | string | — | — | Filter by status (case-insensitive) |
| `limit` | integer | 200 | 1000 | Number of results |
| `offset` | integer | 0 | — | Pagination offset |

**Response**

```json
{
  "total": 806,
  "items": [
    {
      "slug": "11255",
      "title": "Highbridge, United Kingdom",
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
      "slug": "11255",
      "title": "Highbridge, United Kingdom",
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
      "slug": "11255",
      "title": "Highbridge, United Kingdom",
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

### `GET /superchargers/soon/:slug`

Single supercharger with full status history.

**Path parameters**

| Param | Description |
|---|---|
| `slug` | Location slug (stable identifier from Tesla's API) |

**Response**

```json
{
  "slug": "11255",
  "title": "Highbridge, United Kingdom",
  "latitude": 51.22962,
  "longitude": -2.959685,
  "status": "UNDER_CONSTRUCTION",
  "raw_status_value": "Under Construction",
  "tesla_url": "https://www.tesla.com/findus?location=11255",
  "first_seen_at": "2026-03-15T10:30:00Z",
  "last_scraped_at": "2026-03-31T08:45:00Z",
  "is_active": true,
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

`old_status` is `null` for the first-seen entry. `is_active` is `false` when the charger has disappeared from the Tesla feed.

**Errors:** `404` if the slug is not found.

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

Endpoints that support pagination use `limit` and `offset` query parameters. Responses include a `total` field with the full count of matching records regardless of the current page.

```
GET /superchargers/soon?limit=50&offset=100
```
