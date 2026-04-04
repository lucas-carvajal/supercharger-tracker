# Layered Architecture Refactor

## Context

The codebase has a solid foundation but three structural pain points:

1. **`db.rs` is 745 lines** and serves three different consumers with unrelated concerns: scraper reads, CLI reads, API reads, and write operations.
2. **`main.rs` mixes CLI parsing with business logic** — the three workflow bodies contain real orchestration logic that doesn't belong alongside `clap` struct definitions.
3. **Domain objects, scraper code, and utilities are scattered at the top level** with no grouping.

The goal is to introduce clear layers without changing any behavior:
- **`domain/`** — core types, enums, and pure logic (no I/O)
- **`scraper/`** — Tesla API raw types + headless Chrome loader (grouped as they're tightly coupled)
- **`repository/`** — split `db.rs` into focused structs
- **`application/`** — workflow orchestration extracted from `main.rs`
- **`api/`** — already exists; update to use typed state
- **`util/`** — miscellaneous helpers (`display.rs`)

---

## Target Structure

```
src/
  main.rs                   # CLI parsing + dispatch only (~80 lines)

  domain/
    mod.rs
    coming_soon.rs          # ComingSoonSupercharger, SiteStatus, ChargerCategory
    supercharger.rs         # Supercharger, ChargingAccess
    sync.rs                 # SyncPlan, compute_sync(), StatusChange, OpenResult

  scraper/
    mod.rs                  # pub use re-exports (load_from_browser, fetch_open_status_for_ids, etc.)
    raw.rs                  # Tesla API deserialization types (moved from src/raw.rs)
    loaders.rs              # Chrome CDP + API fetching (moved from src/loaders.rs)

  repository/
    mod.rs                  # pub use re-exports
    connection.rs           # connect() only
    supercharger.rs         # SuperchargerRepository struct
    scrape_run.rs           # ScrapeRunRepository struct

  application/
    mod.rs
    scrape.rs               # run_scrape() extracted from main.rs
    status.rs               # run_status() extracted from main.rs
    retry.rs                # run_retry_failed() extracted from main.rs

  util/
    mod.rs
    display.rs              # ASCII table rendering (moved from src/display.rs)

  api/
    mod.rs                  # updated: AppState replacing PgPool in State<>
    superchargers.rs        # updated: State<AppState> + repo method calls
    scrape_runs.rs          # updated: State<AppState> + repo method calls
    regions.rs              # region filter resolution (moved from src/regions.rs)
```

---

## Tables Without Dedicated Repositories: `status_changes` and `opened_superchargers`

These two tables are managed exclusively through `SuperchargerRepository` — they do **not** need their own repository structs:

**`status_changes`**:
- Writes happen inside `save_chargers()` as part of the single atomic transaction (bulk-insert via `unnest`)
- Reads happen via `get_status_history()` (single charger history) and `list_recent_changes()` (LEFT JOINs against both `coming_soon_superchargers` and `opened_superchargers` to get titles for chargers that have since been deleted)
- `get_last_run_stats()` counts rows per run_id (lives in `ScrapeRunRepository` since it's a scrape-run summary)
- Logically part of the `ComingSoonSupercharger` aggregate — never inserted independently

**`opened_superchargers`**:
- Writes happen inside `save_chargers()` — the charger graduation flow (copy from `coming_soon_superchargers` then delete, within the same transaction)
- Read only as a LEFT JOIN target in `list_recent_changes()` to recover title/city/region for graduated chargers
- No standalone read queries exist; it's a write-sink from the application perspective

Both remain entirely within `SuperchargerRepository`. Extracting them into separate repos would require splitting the `save_chargers` transaction across repository boundaries, which would break atomicity guarantees.

---

## Layer Dependency Direction

```
domain      ←  no dependencies (pure types + logic)
scraper     ←  depends on domain (OpenResult, ComingSoonSupercharger constructors)
             note: domain/coming_soon.rs imports scraper::raw types for constructors
             — pragmatic compromise; raw types are pure data with no I/O
repository  ←  depends on domain
application ←  depends on domain + repository + scraper
api         ←  depends on repository
util        ←  depends on domain (for display types)
main        ←  depends on all layers
```

---

## Detailed Plan

### Step 1: Create `domain/`

**`domain/coming_soon.rs`** — move `coming_soon.rs` verbatim. Update its import:
`use crate::raw::{...}` → `use crate::scraper::raw::{...}` (done after scraper/ is created)

**`domain/supercharger.rs`** — move `supercharger.rs` verbatim. Update its import similarly.

**`domain/sync.rs`** — move `sync.rs` verbatim AND add `StatusChange` + `OpenResult` moved from `db.rs`:

```rust
// Added from db.rs:
pub struct StatusChange {
    pub supercharger_id: String,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
}

pub struct OpenResult {
    pub opening_date: Option<NaiveDate>,
    pub num_stalls: Option<i32>,
    pub open_to_non_tesla: Option<bool>,
}
```

**`domain/mod.rs`**:
```rust
pub mod coming_soon;
pub mod supercharger;
pub mod sync;

pub use coming_soon::{ChargerCategory, ComingSoonSupercharger, SiteStatus};
pub use supercharger::{ChargingAccess, Supercharger};
pub use sync::{compute_sync, OpenResult, StatusChange, SyncPlan};
```

### Step 2: Create `scraper/`

**`scraper/raw.rs`** — move `raw.rs` verbatim.
**`scraper/loaders.rs`** — move `loaders.rs` verbatim. Update import:
`use crate::db::OpenResult` → `use crate::domain::OpenResult`

**`scraper/mod.rs`**:
```rust
pub mod loaders;
pub mod raw;

pub use loaders::{fetch_open_status_for_ids, launch_browser_and_wait, load_from_browser};
```

Now fix `domain/coming_soon.rs` and `domain/supercharger.rs`:
`use crate::raw::{...}` → `use crate::scraper::raw::{...}`

Update `main.rs` module declarations: remove `mod coming_soon; mod raw; mod loaders; mod sync;`, add `mod domain; mod scraper;`.

### Step 3: `repository/connection.rs`
Move `connect()` verbatim from `db.rs`.

### Step 4: `repository/supercharger.rs`

```rust
pub struct SuperchargerRepository { pool: PgPool }

impl SuperchargerRepository {
    pub fn new(pool: PgPool) -> Self

    // Scraper reads
    pub async fn get_current_statuses(&self) -> Result<HashMap<String, SiteStatus>>
    pub async fn get_failed_detail_chargers(&self) -> Result<Vec<ComingSoonSupercharger>>
    pub async fn get_failed_open_status_chargers(&self) -> Result<Vec<ComingSoonSupercharger>>

    // CLI reads
    pub async fn get_db_stats(&self) -> Result<DbStats>     // was get_current_db_stats()

    // Write
    pub async fn save_chargers(&self, plan: &SyncPlan, scrape_run_id: i64, ...) -> Result<()>

    // API reads
    pub async fn list_coming_soon(&self, ...) -> Result<(i64, Vec<ApiSupercharger>)>
    pub async fn count_coming_soon_by_status(&self) -> Result<HashMap<String, i64>>
    pub async fn get_coming_soon(&self, id: &str) -> Result<Option<ApiSupercharger>>
    pub async fn get_status_history(&self, id: &str) -> Result<Vec<ApiStatusHistory>>
    pub async fn list_recent_changes(&self, ...) -> Result<(i64, Vec<ApiRecentChange>)>
    pub async fn list_recent_additions(&self, ...) -> Result<(i64, Vec<ApiRecentAddition>)>
}
```

Types `DbStats`, `ApiSupercharger`, `ApiStatusHistory`, `ApiRecentChange`, `ApiRecentAddition` move into this file.

### Step 5: `repository/scrape_run.rs`

```rust
pub struct ScrapeRunRepository { pool: PgPool }

impl ScrapeRunRepository {
    pub fn new(pool: PgPool) -> Self

    pub async fn record_run(&self, ...) -> Result<i64>      // was record_scrape_run()
    pub async fn get_last_run_stats(&self) -> Result<Option<RunStats>>
    pub async fn list_scrape_runs(&self, limit: i64) -> Result<Vec<ApiScrapeRun>>
}
```

Types `RunStats`, `ApiScrapeRun` move into this file.

**`repository/mod.rs`**:
```rust
pub mod connection;
pub mod scrape_run;
pub mod supercharger;

pub use connection::connect;
pub use scrape_run::{ApiScrapeRun, RunStats, ScrapeRunRepository};
pub use supercharger::{ApiRecentAddition, ApiRecentChange, ApiSupercharger,
                       ApiStatusHistory, DbStats, SuperchargerRepository};
```

### Step 6: Delete `db.rs`

### Step 7: Create `util/`
**`util/display.rs`** — move `display.rs` verbatim. Update imports from `crate::` to `crate::domain::`.
**`util/mod.rs`**: `pub mod display;`

### Step 8: `application/scrape.rs`, `status.rs`, `retry.rs`
Move the three workflow bodies from `main.rs` verbatim. New signatures:
```rust
pub async fn run_scrape(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    country: String,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>>

pub async fn run_status(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
) -> Result<(), Box<dyn std::error::Error>>

pub async fn run_retry_failed(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>>
```

### Step 9: Slim down `main.rs`

```rust
let pool = repository::connect(&database_url).await?;
let supercharger_repo = repository::SuperchargerRepository::new(pool.clone());
let scrape_run_repo = repository::ScrapeRunRepository::new(pool.clone());

match args.command {
    Command::Scrape { country, show_browser } =>
        application::scrape::run_scrape(&supercharger_repo, &scrape_run_repo, country, show_browser).await?,
    Command::Status =>
        application::status::run_status(&supercharger_repo, &scrape_run_repo).await?,
    Command::RetryFailed { show_browser } =>
        application::retry::run_retry_failed(&supercharger_repo, &scrape_run_repo, show_browser).await?,
    Command::Host { port } =>
        run_host(pool, port).await?,
}
```

### Step 10: Update `api/`

**`api/regions.rs`** — move `regions.rs` verbatim (no import changes needed).

**`api/mod.rs`** — add `mod regions;`, replace `State<PgPool>` with `State<AppState>`:
```rust
#[derive(Clone)]
pub struct AppState {
    pub supercharger: SuperchargerRepository,
    pub scrape_run: ScrapeRunRepository,
}

pub fn router(pool: PgPool) -> Router {
    let state = AppState {
        supercharger: SuperchargerRepository::new(pool.clone()),
        scrape_run: ScrapeRunRepository::new(pool),
    };
    Router::new()
        ...
        .with_state(state)
}
```

Update `api/superchargers.rs`: `State<PgPool>` → `State<AppState>`, call `state.supercharger.*` methods.
Update `api/scrape_runs.rs`: `State<PgPool>` → `State<AppState>`, call `state.scrape_run.*` methods.

---

## Critical Files

| File | Change |
|---|---|
| `src/db.rs` | Deleted after full migration |
| `src/coming_soon.rs` | → `src/domain/coming_soon.rs` (domain type — name stays, only path changes) |
| `src/supercharger.rs` | → `src/domain/supercharger.rs` |
| `src/sync.rs` | → `src/domain/sync.rs` + gains `StatusChange`/`OpenResult` from db.rs |
| `src/raw.rs` | → `src/scraper/raw.rs` |
| `src/loaders.rs` | → `src/scraper/loaders.rs`; fix one import |
| `src/display.rs` | → `src/util/display.rs` |
| `src/regions.rs` | → `src/api/regions.rs` |
| `src/main.rs` | Module decls updated; workflow bodies extracted |
| `src/api/mod.rs` | Introduce `AppState`, add `mod regions;` |
| `src/api/superchargers.rs` | `State<PgPool>` → `State<AppState>` + method calls |
| `src/api/scrape_runs.rs` | `State<PgPool>` → `State<AppState>` + method call |

---

## Execution Order (keeps the branch compilable at each step)

1. Create `domain/` — move `coming_soon.rs`, `supercharger.rs`; create `domain/sync.rs` with moved `StatusChange`/`OpenResult` from db.rs; update `main.rs` module decls → compile
2. Create `scraper/` — move `raw.rs` and `loaders.rs`; fix `loaders.rs` import; fix `domain/coming_soon.rs` + `domain/supercharger.rs` imports; update `main.rs` → compile
3. Create `repository/connection.rs` — move `connect()` → compile
4. Create `repository/supercharger.rs` — move functions, update all callers → compile
5. Create `repository/scrape_run.rs` — move remaining db.rs functions → compile
6. Delete `db.rs` → compile
7. Create `util/display.rs` — move display.rs → compile
8. Move `regions.rs` → `api/regions.rs`, add `mod regions;` to `api/mod.rs` → compile
9. Create `application/` modules — move workflow bodies from `main.rs` → compile
10. Slim down `main.rs` → compile
11. Update API layer to use `AppState` → compile
12. Update docs: `CLAUDE.md` and save plan to `docs/plans/`

---

## Step 12: Update Documentation

### `CLAUDE.md` — "Project Structure"
Replace the flat file list with the new layered structure:
```
src/
  main.rs              # CLI definition and subcommand dispatch
  domain/
    coming_soon.rs     # ComingSoonSupercharger, SiteStatus, ChargerCategory
    supercharger.rs    # Open (live) supercharger type
    sync.rs            # Diff logic: compute_sync, SyncPlan, StatusChange, OpenResult
  scraper/
    raw.rs             # Raw Tesla API deserialization types
    loaders.rs         # Data loading: headless Chrome via CDP
  repository/
    connection.rs      # Database connection and migrations
    supercharger.rs    # SuperchargerRepository: charger reads, writes, status history
    scrape_run.rs      # ScrapeRunRepository: run history reads and writes
  application/
    scrape.rs          # Scrape workflow orchestration
    status.rs          # Status display workflow
    retry.rs           # Retry-failed workflow
  util/
    display.rs         # Terminal table rendering
  api/
    mod.rs             # Axum router setup, AppState, error handling
    superchargers.rs   # Supercharger API endpoints
    scrape_runs.rs     # Scrape history endpoints
    regions.rs         # Region filter resolution
```

**"Architecture Notes / Database Schema"** — update to reflect four tables (not three):
- Add `opened_superchargers` to the table list
- Note that `status_changes` has no FK to `coming_soon_superchargers` (dropped in migration to support history surviving charger deletion)
- Clarify that `status_changes` and `opened_superchargers` are managed through `SuperchargerRepository`

Also save the refactor plan under `docs/plans/` per project conventions.

---

## Verification

```bash
cargo build          # must compile cleanly
cargo test --verbose # domain::sync tests must all pass
cargo clippy         # no new warnings
```

---

## What This Does NOT Change

- Zero behavior changes — pure reorganization
- No new abstractions (no traits, no generics, no DI)
- `save_chargers` signature is unchanged (long but deliberate — complex transaction with distinct inputs)
