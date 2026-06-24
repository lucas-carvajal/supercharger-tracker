# Architecture

This document is the single reference for understanding `supercharger-tracker` end to end:
what it does, how the code is organized, how data flows through each subcommand, and how
the data model and deployment fit together. For the HTTP contract see [`API.md`](API.md);
for the day-to-day operator runbook see [`SCRAPE_EXPORT_FLOW.md`](SCRAPE_EXPORT_FLOW.md).

---

## 1. What the service does

Tesla publishes an internal "Find Us" API that backs the map at `tesla.com/findus`. Among
the live Superchargers it also returns **coming-soon** locations (planned and
under-construction sites) and **contest winner** sites. This tool:

1. **Scrapes** that feed worldwide, working around Akamai Bot Manager protection by driving
   a real headless Chrome and issuing `fetch()` calls from inside the page (so the TLS
   fingerprint and Akamai cookies are genuine).
2. **Tracks state over time** — it diffs each scrape against the database and records every
   status transition (first-seen → in-development → under-construction → opened/removed) in
   an append-only audit log.
3. **Detects graduation** — when a coming-soon site disappears from the feed, it actively
   checks Tesla's open-supercharger endpoint to distinguish *opened* from *removed*.
4. **Serves the data** through a read-only REST API (CORS-enabled JSON).
5. **Ships data to prod** via a local→prod export/import pipeline, because scraping happens
   on a VPN-gated local machine and the public API runs elsewhere.

It is a single Rust binary with subcommands; there is no long-running scraper process —
scrapes are invoked manually (or by a scheduler) and the `host` subcommand runs the API.

---

## 2. Tech stack

| Layer | Choice |
|---|---|
| Language | Rust (edition 2024, stable) |
| Async runtime | Tokio 1.x |
| HTTP server | Axum 0.8 + Tower-HTTP 0.6 (CORS, tracing) |
| Database | PostgreSQL 16 (13+ compatible) via SQLx 0.8 (compile-time migrations) |
| CLI | Clap 4.x (derive, env-var support) |
| Browser automation | Chromiumoxide 0.9 (Chrome DevTools Protocol) |
| Serialization | Serde / serde_json |
| Config | Dotenvy 0.15 (`.env`) |
| Logging | `tracing` + `tracing-subscriber` (JSON output) |

---

## 3. Code layout & layering

The codebase is organized into layers with a deliberate dependency direction
(`domain` is pure and depends on nothing app-specific; everything else depends inward):

```
main.rs ──► application/ ──► repository/  ──► Postgres
                │      └────► scraper/     ──► headless Chrome ──► Tesla API
                └──────────► domain/       (pure types + diff logic, no I/O)
api/ ──► repository/ + application/import   (the `host` server)
export.rs                                   (wire types shared by export + import)
util/                                       (config, display helpers)
```

| Module | Responsibility |
|---|---|
| `main.rs` | CLI definition (Clap), env/DB bootstrap, subcommand dispatch, API server launch, graceful shutdown, JSON tracing setup. |
| `domain/` | **Pure** business types and logic. No DB, no network. `coming_soon.rs` (`ComingSoonSupercharger`, `SiteStatus`, `ChargerCategory`), `supercharger.rs` (open-charger type), `sync.rs` (`compute_sync` diff engine — fully unit-tested). |
| `scraper/` | `raw.rs` (raw Tesla JSON deserialization), `loaders.rs` (Chrome launch, Akamai wait, in-browser `fetch` orchestration, batching, retries, failure classification). |
| `repository/` | DB access. `connection.rs` (pool + `sqlx::migrate!`), `supercharger.rs` (`SuperchargerRepository`: reads/writes/history/atomic save), `scrape_run.rs` (`ScrapeRunRepository`: run history), `models.rs` (query-result structs). |
| `application/` | Workflow orchestration, one file per subcommand: `scrape`, `status`, `retry`, `export_diff`, `export_snapshot`, plus `import` (shared by the HTTP handler). |
| `api/` | Axum router + handlers: `superchargers.rs`, `scrape_runs.rs`, `regions.rs` (region-filter resolution), `import.rs` (admin import endpoints). `mod.rs` holds `AppState`, routing, and `ApiError`. |
| `export.rs` | `ScrapeExport` wire format (`DiffExport` / `SnapshotExport`) — the contract between `export-*` (producer) and `import` (consumer). |
| `util/` | `config.rs` (env loading — the only place env vars are read), `display.rs` (terminal tables, currently unused). |

---

## 4. Domain model & key concepts

### 4.1 Identity

Each location is identified by its **Tesla location URL slug** (e.g. `"11255"` from
`https://www.tesla.com/findus?location=11255`). This slug is stable across scrapes and is
the primary key (`id`) everywhere. Tesla's internal `uuid` field is intentionally **ignored**
— it changes arbitrarily for the same physical site and is unreliable as an identifier.
Locations whose slug is empty or `"null"` are skipped (no stable identity → untrackable).

### 4.2 Status lifecycle (`SiteStatus`)

```
                 (first seen)
                      │
          ┌───────────▼────────────┐
          │       PRELIMINARY        │◄──────────┐
          └───────────┬────────────┘            │ (reappears from a
                      │                          │  REMOVED tombstone)
          ┌───────────▼────────────┐            │
          │          DESIGN          │            │
          └───────────┬────────────┘            │
                      │                          │
          ┌───────────▼────────────┐            │
          │      CONSTRUCTION        │            │
          └───────────┬────────────┘            │
                      │ disappears from feed     │
            ┌─────────┴──────────┐               │
            │  open-check probe  │               │
            ▼                    ▼               │
        ┌────────┐          ┌─────────┐          │
        │ OPENED │          │ REMOVED │──────────┘
        └────────┘          └─────────┘
   (row copied to          (tombstone kept in
   opened_superchargers     coming_soon_superchargers
   then deleted)            so reappearance is a
                            transition, not a new entry)
```

| Status | Meaning |
|---|---|
| `PRELIMINARY` | Earliest build stage (Tesla `project_status`: "Preliminary"; also the fallback for customer-facing "In Development"). |
| `DESIGN` | Planning underway (Tesla `project_status`: "Design"). |
| `CONSTRUCTION` | Actively being built (Tesla `project_status`: "Construction"; fallback for "Under Construction"). |
| `UNKNOWN` | Details fetch failed, or Tesla returned an unrecognized status string. |
| `REMOVED` | Disappeared from the feed and confirmed *not* open. Kept as a **tombstone** row so that if the site reappears, it records a `REMOVED → PRELIMINARY/DESIGN/CONSTRUCTION` transition instead of a spurious first-appearance. |
| `OPENED` | Confirmed live via the open-check endpoint. Recorded in `status_changes`, then the row is **copied** to `opened_superchargers` and **deleted** from `coming_soon_superchargers`. |

Status is stored as `TEXT` (not a Postgres enum), derived from Tesla `project_status` on each scrape.

### 4.3 Charger category (`ChargerCategory`)

Derived from Tesla's `location_type` array: `CURRENT_WINNER` and `WINNER` (charging-contest
winner sites) take precedence over the default `COMING_SOON`. A location counts as
coming-soon if its type includes `coming_soon_supercharger`, `winner_supercharger`, or
`current_winner_supercharger`.

### 4.4 Title parsing

Tesla titles are `"City, Region"`. `parse_title` splits on the **last** comma; if there is
no comma or either side is empty, both `city` and `region` are `null`. The detail endpoint's
`coming_soon_name` is preferred over the raw location title when available.

---

## 5. Database schema

Migrations run automatically on startup via `sqlx::migrate!()`. Four tables, two enums.

| Table | Purpose | Owner repo |
|---|---|---|
| `scrape_runs` | One row per execution: country, timestamp, total count, failure counters, `run_type` (`full`/`retry`), retry counters. `id` is `BIGSERIAL` and doubles as the **import version** (see §8). | `ScrapeRunRepository` |
| `coming_soon_superchargers` | Current state, one row per active/tombstoned site. PK = slug. Holds status, coordinates, `raw_status_value`, `first_seen_at`, `last_scraped_at`, and two failure flags: `details_fetch_failed`, `open_status_check_failed`. | `SuperchargerRepository` |
| `status_changes` | Append-only audit log of **every** transition, including first-seen (`old_status = NULL`). **No FK** to `coming_soon_superchargers` so history survives graduation/deletion. References `scrape_runs(id)`. | `SuperchargerRepository` |
| `opened_superchargers` | Graduated sites confirmed open: opening date, stall count, non-Tesla access. | `SuperchargerRepository` |

Indexes target the API's hot paths: `status_changes(changed_at DESC)` and a partial index for
recent-changes feeds, `coming_soon_superchargers(status)`, `(region)`, `(first_seen_at DESC)`,
and partial indexes on the two `*_failed = TRUE` flags (so `retry-failed` scans are cheap).

### Atomicity

The core write, `SuperchargerRepository::save_chargers()`, runs in a **single transaction**:
upserts, unchanged-row touch-ups (`last_scraped_at`), status-change inserts, the graduation
flow (copy to `opened_superchargers` → delete from `coming_soon_superchargers`), removed
tombstones, and the failure flags all commit together or not at all. The import path
(`save_chargers_from_diff`, `apply_snapshot`) is likewise transactional.

---

## 6. Subcommands & data flows

`main.rs` parses args, loads `Config`, opens the pool, constructs both repositories, and
dispatches. Each subcommand maps to one `application/` workflow.

### 6.1 `scrape` — full sync (`application/scrape.rs`)

```
launch_browser_and_wait ──► load_from_browser ──► compute_sync ──► open-check ──► save_chargers
   (Chrome + Akamai)         (locations+details)   (pure diff)     (disappeared)   (1 txn)
```

1. **Launch & authenticate** (`launch_browser_and_wait`): find Chrome, spawn a fresh-profile
   headless (or `--show-browser`) instance, navigate to `tesla.com/findus`, wait ~8s for
   Akamai scripts to settle, then poll the locations endpoint until it returns JSON (ready)
   rather than an HTML block page. Headless timeout 30s; visible 180s (lets a human solve a
   challenge).
2. **Load locations** (`load_from_browser`): one in-browser `fetch` of
   `/api/findus/get-locations?country=US` (US returns worldwide data). If the response is
   HTML, Akamai is still blocking → abort.
3. **Fetch details**: for every coming-soon slug, fetch `get-location-details?...&functionTypes=coming_soon_supercharger`
   in batches of 5 (1.2s between batches, 10s per-request timeout, one retry with backoff per
   failed batch). Failures are classified (`timeout`, `http_error:NNN`, `html_block`,
   `json_parse`, …); HTML blocks abort remaining batches. IDs that genuinely fail land in
   `failed_detail_ids`.
4. **Diff** (`compute_sync`, pure/tested): compares DB statuses against the fresh scrape and
   produces a `SyncPlan` of `upserts`, `unchanged`, `status_changes`, and `disappeared_ids`.
   Key rule: **if a detail fetch failed for an existing charger, its current DB status is
   preserved** so a transient failure never records a false `→ UNKNOWN` transition. REMOVED
   tombstones absent from the feed are excluded from `disappeared_ids` (no repeated re-checks).
5. **Open-check** (`fetch_open_status_for_ids`): for disappeared sites, probe
   `...&functionTypes=supercharger`. `site_status == "open"` → graduate (capture opening date,
   stalls, non-Tesla access). A clean 404 → confirmed absent → **REMOVED**. A fetch failure →
   flagged (`open_status_check_failed`) for `retry-failed`, *not* marked removed. A total call
   failure flags **all** disappeared sites so none are wrongly removed.
6. **Persist**: record the `scrape_runs` row (`run_type = "full"`), then one atomic
   `save_chargers` applying the whole plan.

The browser session is always closed afterward (success or error).

### 6.2 `retry-failed` (`application/retry.rs`)

Re-runs only the failed parts of the **most recent** scrape — it does **not** download the
full locations list, and it attributes any new status changes to that existing
`scrape_runs` row (`parent_run_id`), bumping its retry counters rather than creating a new run.

- **Phase 1 — detail retries**: re-fetch details for `details_fetch_failed = TRUE` chargers
  (batched), apply `with_details`, diff per batch, save. Counters in the DB are refreshed after
  each batch. A block response skips remaining batches.
- **Phase 2 — open-status retries**: re-probe `open_status_check_failed = TRUE` chargers and
  resolve each to opened / still-failing / removed. Skipped if Phase 1 was blocked.

### 6.3 `status` (`application/status.rs`)

Prints a summary of the latest run (timestamp, counts, failures, status-change count) and
current DB state. Read-only.

### 6.4 `export-diff` / `export-snapshot` (`application/export_diff.rs`, `export_snapshot.rs`)

- **`export-diff`** writes `ScrapeExport::Diff` for the latest run: changed chargers, all
  status changes for the run, graduated (opened) chargers, and removed IDs, tagged with the
  local `run_id`. Refuses to export if the run still has unresolved failures unless `--force`.
- **`export-snapshot`** writes `ScrapeExport::Snapshot`: the full contents of all four tables
  (with original IDs and timestamps) for bootstrapping or recovering a prod instance.

### 6.5 `host` (`api/`)

Builds the Axum router with `AppState` (pool + both repos + optional import secret), binds
`0.0.0.0:PORT` (default 8080, or `PORT`/`--port`), serves with graceful shutdown on
Ctrl-C/SIGTERM. CORS is permissive; every request is traced.

---

## 7. Scraper internals (Akamai bypass)

The crux of the design: Tesla's API sits behind Akamai Bot Manager, which fingerprints TLS
and requires JS-generated cookies. Rather than reverse-engineer that, the tool **borrows a
real browser**:

- A fresh Chrome profile (temp dir, auto-cleaned) navigates to `tesla.com/findus`.
- All API calls are `page.evaluate("fetch(...)")` executed **inside** the page, so they carry
  the page's genuine TLS fingerprint, cookies, and headers.
- An initial ~8s settle plus a readiness poll ensures Akamai cookies exist before real work.
- Every in-browser fetch wraps its result as `{ok, data, blocked, error, status}` so Rust can
  distinguish a true failure from a legitimate empty response, and detect HTML block pages
  (response starts with `<`) to abort early instead of hammering a blocked endpoint.
- Batching (size 5), inter-batch delays (1.2s), per-request timeouts (10s), and a single
  retry-with-backoff per failed batch keep request volume polite and resilient.

Three Tesla endpoints are used, all under `/api/findus/`:

| Endpoint | Purpose |
|---|---|
| `get-locations?country=US` | The worldwide location list (US code returns everything). |
| `get-location-details?...&functionTypes=coming_soon_supercharger` | Per-site coming-soon name + status string. |
| `get-location-details?...&functionTypes=supercharger` | Open-check: is a disappeared site now live? Opening date, stalls, access. |

---

## 8. Local → prod data pipeline

Scraping requires a specific network position (VPN-gated), so it runs on a local machine while
the public API runs in prod. Data moves as JSON export files imported over an authenticated
admin endpoint.

```
   LOCAL (scraper)                          PROD (API)
   ──────────────                           ──────────
   scrape / retry-failed
        │
        ▼
   export-diff  ──► scrape_export_{id}.json ──┐
   export-snapshot ──► snapshot.json ─────────┤  POST /admin/import/scrapes
                                              │  (X-Admin-Internal-Secret)
                                              ▼
                                         apply_import
                                         ├─ Diff:     dedup + ordering + atomic apply
                                         └─ Snapshot: TRUNCATE + restore all 4 tables
```

**Versioning & ordering.** `scrape_runs.id` is the import version. A diff carries its local
`run_id`; prod inserts that row id verbatim (`OVERRIDING SYSTEM VALUE`). Import is:

- **idempotent** — a `run_id` already present returns `duplicate` (no-op);
- **strictly ordered** — a diff must have `run_id == MAX(scrape_runs.id) + 1`, else
  `out_of_order` (409). `--force` / `?force=true` bypasses the check for gap recovery.

**Bootstrap rule.** On an empty prod DB, `MAX(id)` is 0, so the ordering check expects
`run_id = 1` — which never matches a real local run (local IDs start much higher). Therefore
**always apply a snapshot to a fresh prod instance before importing diffs.** Snapshot import
restores all four tables (original IDs preserved) and resets the sequence; the first subsequent
diff must be `MAX(restored id) + 1`.

`GET /admin/import/current-version` reports prod's `MAX(scrape_runs.id)` and the next expected
version, so the local side knows which diff to send next.

---

## 9. HTTP API surface (summary)

Read-only, JSON, CORS-permissive. Full reference in [`API.md`](API.md).

| Method & path | Purpose |
|---|---|
| `GET /health` | DB-backed liveness check. |
| `GET /superchargers/soon` | Paginated list, filterable by `status` and `region`. |
| `GET /superchargers/soon/map` | Lightweight markers (flat array). |
| `GET /superchargers/soon/stats` | Counts by status + `as_of` timestamp. |
| `GET /superchargers/soon/recent-changes` | Recent transitions (excludes first-seen and `→ UNKNOWN`). |
| `GET /superchargers/soon/recent-additions` | Recently first-seen sites. |
| `GET /superchargers/soon/:id` | One site + full status history. |
| `GET /scrape-runs` | Recent run records. |
| `POST /admin/import/scrapes` | Apply a diff/snapshot (auth required). |
| `GET /admin/import/current-version` | Current/next import version (auth required). |

**Region resolution** (`api/regions.rs`) maps a single `?region=` input to one or more DB
region strings, handling country aggregates (`US` → all states), spelling variants
(`Türkiye`/`Turkey`), and the `NT` collision (Australian NT vs Canadian NWT). Unknown values
→ `400`. Errors are uniform JSON (`{ "error": ... }`) via `ApiError` → `400/404/500`.

---

## 10. Configuration & operations

| Env var | Required | Purpose |
|---|---|---|
| `DATABASE_URL` | Yes | Postgres connection string. |
| `RUST_INTERNAL_IMPORT_SECRET` | For import | Shared secret for `X-Admin-Internal-Secret` on admin import endpoints. Unset → those endpoints return `503`. |
| `DB_MAX_CONNECTIONS` | No | Pool size (default 10). |
| `PORT` | No | API port (default 8080; `--port` overrides). |

All env access is centralized in `util/config.rs` — add new vars there. Logs are structured
JSON via `tracing` (`RUST_LOG` honored; chromiumoxide handler noise suppressed to `error`).

**Build & test:** `cargo build --release`, `cargo test`. CI (`.github/workflows/rust.yml`)
builds and tests on push/PR to `main`. The `compute_sync` diff engine and scraper
failure-classification helpers carry the bulk of the unit tests. Run `cargo fmt` and
`cargo clippy` before committing.

---

## 11. Design principles worth remembering

- **Pure core, impure shell.** `domain/` (especially `compute_sync`) is side-effect-free and
  unit-tested; all I/O lives in `scraper`/`repository`/`api`. Diff bugs are caught without a
  browser or DB.
- **Failures are first-class.** A transient fetch failure must never look like a real status
  change. Failed details preserve the existing status; failed open-checks defer the
  removed/opened decision to `retry-failed` instead of guessing.
- **History is immutable.** `status_changes` is append-only with no FK, so a site's full
  timeline survives graduation, deletion, and reappearance.
- **Tombstones over deletion.** REMOVED rows are kept so reappearance is a real transition.
- **Atomic writes.** Every multi-table mutation is one transaction.
- **Deterministic, idempotent transfer.** Sequential, dedup-checked, ordered diff imports
  with snapshot bootstrap make local→prod replication safe to retry.
</content>
</invoke>
