# Integration Tests Plan: supercharger-tracker

## Context

The project currently has 6 pure-function unit tests in `sync.rs` but zero integration tests. Any change to DB queries, API handlers, scrape-flow wiring, or the schema can silently break production behaviour. This plan adds integration tests for every key flow so regressions surface immediately in CI.

---

## Key Flows to Cover

| Flow | Entry points | What can break |
|---|---|---|
| **DB – save_chargers** | `db::save_chargers` | Wrong upsert, missing status_changes row, first_seen_at overwritten, is_active not set |
| **DB – read queries** | `db::get_current_statuses`, `db::get_failed_detail_chargers`, `db::list_coming_soon`, etc. | Wrong filters (inactive included, failed not returned), wrong counts |
| **Scrape flow** | `load_from_file` → `compute_sync` → `db::save_chargers` | End-to-end DB state after first / second scrape |
| **API – list/filter/paginate** | `GET /superchargers/soon` | Status filter validation, limit/offset clamping, tesla_url computed correctly |
| **API – stats** | `GET /superchargers/soon/stats` | Counts grouped by status, as_of timestamp |
| **API – detail** | `GET /superchargers/soon/{uuid}` | 404 for unknown uuid, status history included and ordered |
| **API – recent-changes** | `GET /superchargers/soon/recent-changes` | Only rows where old_status IS NOT NULL (excludes first appearances) |
| **API – recent-additions** | `GET /superchargers/soon/recent-additions` | Ordered by first_seen_at DESC, is_active=true only |
| **API – scrape-runs** | `GET /scrape-runs` | Ordered DESC by scraped_at, limit respected |

---

## Structural Change Required: Add `src/lib.rs`

Tests in `tests/*.rs` are a separate compilation unit and cannot access private modules of a binary crate. The standard Rust fix is to extract a library crate:

1. **Create `src/lib.rs`** – re-export every module as `pub mod`.
2. **Modify `src/main.rs`** – remove the `mod x;` declarations and replace with `use tesla_superchargers::{api, coming_soon, db, loaders, sync};`.

This has zero runtime effect; `main.rs` remains the entry point. `src/lib.rs` simply makes the internal modules reachable from `tests/`.

**`src/lib.rs`** (new file):
```rust
pub mod api;
pub mod coming_soon;
pub mod db;
pub mod display;       // if it exists
pub mod loaders;
pub mod raw;
pub mod supercharger;  // if it exists
pub mod sync;
```

**`src/main.rs`** (modified): replace the `mod x;` block at the top with:
```rust
use tesla_superchargers::{api, coming_soon, db, loaders, sync};
```

---

## Cargo.toml Changes

```toml
[lib]
name = "tesla_superchargers"
path = "src/lib.rs"

[[bin]]
name = "tesla-superchargers"
path = "src/main.rs"

[dev-dependencies]
# tokio and serde_json are already in [dependencies]; repeating here for clarity
tokio       = { version = "1", features = ["full"] }
serde_json  = "1"
tower       = { version = "0.4", features = ["util"] }   # ServiceExt::oneshot for API tests
http-body-util = "0.1"                                    # reading Axum response bodies in tests
```

---

## New Files

```
tests/
  db_tests.rs                     # DB layer integration tests
  scrape_flow_tests.rs            # End-to-end scrape pipeline tests
  api_tests.rs                    # HTTP API integration tests
  fixtures/
    scrape_initial.json           # First scrape: 3 coming-soon chargers
    scrape_updated.json           # Second scrape: 2 remain, 1 disappears
```

---

## Test Fixture JSON Format

`raw::ApiResponse` deserializes as `{ "data": { "data": [...] } }`.  
`is_coming_soon()` requires `location_type` to contain `"coming_soon_supercharger"`.  
Status is always `Unknown` in file mode (no details fetch).

**`tests/fixtures/scrape_initial.json`**:
```json
{
  "data": {
    "data": [
      { "uuid": "uuid-1", "title": "Alpha SC", "latitude": 1.0, "longitude": 1.0,
        "location_type": ["coming_soon_supercharger"], "location_url_slug": "alpha-sc" },
      { "uuid": "uuid-2", "title": "Beta SC",  "latitude": 2.0, "longitude": 2.0,
        "location_type": ["coming_soon_supercharger"], "location_url_slug": "beta-sc" },
      { "uuid": "uuid-3", "title": "Gamma SC", "latitude": 3.0, "longitude": 3.0,
        "location_type": ["coming_soon_supercharger"], "location_url_slug": "gamma-sc" }
    ]
  }
}
```

**`tests/fixtures/scrape_updated.json`** – uuid-2 removed (disappears):
```json
{
  "data": {
    "data": [
      { "uuid": "uuid-1", "title": "Alpha SC", "latitude": 1.0, "longitude": 1.0,
        "location_type": ["coming_soon_supercharger"], "location_url_slug": "alpha-sc" },
      { "uuid": "uuid-3", "title": "Gamma SC", "latitude": 3.0, "longitude": 3.0,
        "location_type": ["coming_soon_supercharger"], "location_url_slug": "gamma-sc" }
    ]
  }
}
```

---

## `tests/db_tests.rs` — DB Layer

Uses `#[sqlx::test]` — each test receives a fresh `PgPool` with migrations already applied, and the database is dropped automatically after the test.

Shared helper:
```rust
fn make_charger(uuid: &str, status: SiteStatus) -> ComingSoonSupercharger { ... }
```

| Test | Setup | Action | Assert |
|---|---|---|---|
| `new_chargers_inserted` | empty DB | `save_chargers` with 2 upserts + 2 status_changes (old=None) | 2 rows in chargers table; is_active=true; status_changes has 2 rows with old_status NULL |
| `first_seen_at_never_updated` | 1 charger saved | save same uuid again as upsert | first_seen_at unchanged; last_scraped_at updated |
| `status_change_recorded` | 1 charger IN_DEVELOPMENT | save same uuid as UNDER_CONSTRUCTION upsert + status_change | status_changes row has correct old/new; charger row updated |
| `disappeared_charger_marked_inactive` | 2 chargers saved | save with `disappeared_uuids=[uuid-2]` | uuid-2 is_active=FALSE; uuid-1 unchanged |
| `inactive_excluded_from_current_statuses` | 1 active + 1 inactive | call `get_current_statuses` | returns only the active charger |
| `details_fetch_failed_flag_set` | empty DB | save charger with its slug in `failed_detail_slugs` | `details_fetch_failed=TRUE` in DB |
| `get_failed_detail_chargers_correct_filter` | 3 chargers: active+failed, active+ok, inactive+failed | call `get_failed_detail_chargers` | returns only the active+failed one |
| `unchanged_uuids_touch_last_scraped_at` | 1 charger | save with `unchanged_uuids=[uuid]` | last_scraped_at updated; no new status_changes row |

---

## `tests/scrape_flow_tests.rs` — End-to-End Scrape Pipeline

Uses `#[sqlx::test]`. Shared helper replicates `run_scrape()` from `main.rs`:

```rust
async fn run_scrape_from_file(pool: &PgPool, path: &str) {
    let result = loaders::load_from_file(path).await.unwrap();
    let coming_soon: Vec<_> = result.locations.iter()
        .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
        .map(|l| ComingSoonSupercharger::from_location(l, None))
        .collect();
    let run_id = db::record_scrape_run(pool, "US", coming_soon.len() as i32, 0, "full").await.unwrap();
    let current = db::get_current_statuses(pool).await.unwrap();
    let plan = sync::compute_sync(current, &coming_soon, &HashSet::new());
    db::save_chargers(pool, &plan.upserts, &plan.unchanged_uuids,
        &plan.status_changes, &plan.disappeared_uuids, run_id, &HashSet::new()).await.unwrap();
}
```

| Test | Steps | Assert |
|---|---|---|
| `first_scrape_inserts_all_chargers` | `scrape_initial.json` on empty DB | 3 chargers in DB; 3 status_changes with old_status NULL; 1 scrape_run row; all is_active=true |
| `second_scrape_unchanged_no_new_events` | `scrape_initial.json` twice | after 2nd: still 3 chargers; no new status_changes rows; last_scraped_at updated |
| `disappeared_charger_goes_inactive` | initial then updated fixture | uuid-2 is_active=FALSE; uuid-1 and uuid-3 still active |
| `scrape_run_recorded_correctly` | `scrape_initial.json` | scrape_runs row: total_count=3, run_type='full', country='US' |
| `status_transition_recorded` | insert charger IN_DEVELOPMENT directly; run compute_sync + save with UNDER_CONSTRUCTION | status_changes row with old=IN_DEVELOPMENT, new=UNDER_CONSTRUCTION; charger status updated |
| `failed_detail_flag_preserved` | insert charger; save with its slug in failed_detail_slugs + charger in unchanged_uuids | details_fetch_failed=TRUE; status unchanged; no spurious status_change row |

---

## `tests/api_tests.rs` — HTTP API

Uses `#[sqlx::test]` for DB isolation. Calls the Axum router via `tower::ServiceExt::oneshot` — no real TCP server needed.

```rust
async fn get_json(pool: PgPool, uri: &str) -> (StatusCode, serde_json::Value) {
    let app = api::router(pool);
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
```

| Test | DB seed | Request | Assert |
|---|---|---|---|
| `list_empty_db` | empty | `GET /superchargers/soon` | `{total:0, items:[]}` |
| `list_returns_only_active` | 2 active + 1 inactive | `GET /superchargers/soon` | total=2; inactive not in items |
| `list_status_filter` | 1 IN_DEVELOPMENT + 1 UNDER_CONSTRUCTION | `GET /superchargers/soon?status=IN_DEVELOPMENT` | total=1; item status matches |
| `list_invalid_status_returns_400` | any | `GET /superchargers/soon?status=BOGUS` | HTTP 400 |
| `list_pagination` | 5 chargers | `GET /superchargers/soon?limit=2&offset=2` | total=5; 2 items returned |
| `list_tesla_url_computed` | 1 charger with slug `test-sc` | `GET /superchargers/soon` | tesla_url = `https://www.tesla.com/findus?location=test-sc` |
| `stats_empty_db` | empty | `GET /superchargers/soon/stats` | total_active=0; all by_status values=0; as_of=null |
| `stats_with_data` | 2 IN_DEV + 1 UNDER_CONST + 1 scrape_run | `GET /superchargers/soon/stats` | total_active=3; counts correct; as_of present |
| `detail_not_found` | empty | `GET /superchargers/soon/no-such-uuid` | HTTP 404 |
| `detail_found_with_history` | 1 charger + 2 status_changes rows | `GET /superchargers/soon/uuid-1` | status_history has 2 entries ordered ASC by changed_at |
| `recent_changes_excludes_first_appearances` | 1 old_status=NULL + 1 old_status=IN_DEV | `GET /superchargers/soon/recent-changes` | total=1; item has old_status=IN_DEVELOPMENT |
| `recent_additions_ordered` | 3 chargers with different first_seen_at | `GET /superchargers/soon/recent-additions` | items ordered newest first |
| `scrape_runs_with_limit` | 5 runs | `GET /scrape-runs?limit=3` | 3 items; ordered newest first |

---

## CI Workflow Changes

Update `.github/workflows/rust.yml` to add a Postgres service and `DATABASE_URL`. The `#[sqlx::test]` macro reads `DATABASE_URL` at runtime, creates per-test databases (`supercharger_test_<test_name>`), runs migrations, and drops them automatically.

```yaml
jobs:
  build:
    runs-on: ubuntu-latest

    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_DB: supercharger_test
          POSTGRES_PASSWORD: pass
        ports:
          - 5432:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5

    env:
      DATABASE_URL: postgres://postgres:pass@localhost:5432/supercharger_test

    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: cargo build --verbose
      - name: Run tests
        run: cargo test --verbose
```

---

## Summary of All Files to Create / Modify

| File | Action |
|---|---|
| `src/lib.rs` | **Create** — `pub mod` re-exports for all modules |
| `src/main.rs` | **Modify** — swap `mod x;` declarations for `use tesla_superchargers::x;` |
| `Cargo.toml` | **Modify** — add `[lib]`, `[[bin]]`, `[dev-dependencies]` |
| `.github/workflows/rust.yml` | **Modify** — add Postgres service + `DATABASE_URL` env |
| `tests/db_tests.rs` | **Create** |
| `tests/scrape_flow_tests.rs` | **Create** |
| `tests/api_tests.rs` | **Create** |
| `tests/fixtures/scrape_initial.json` | **Create** |
| `tests/fixtures/scrape_updated.json` | **Create** |

---

## Verification

After implementation, with a running Postgres:

```bash
DATABASE_URL=postgres://postgres:pass@localhost:5432/supercharger_test cargo test --verbose
# Expected: 6 existing sync unit tests + ~25 new integration tests pass
```

In CI: push to branch → GitHub Actions runs with the Postgres service → all tests must pass before merging to main.
