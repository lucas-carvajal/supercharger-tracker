# Postgres Integration Plan

## Schema

```sql
CREATE TYPE site_status AS ENUM (
    'IN_DEVELOPMENT',
    'UNDER_CONSTRUCTION',
    'UNKNOWN'
);

CREATE TABLE scrape_runs (
    id          BIGSERIAL PRIMARY KEY,
    country     TEXT NOT NULL,
    scraped_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    total_count INT,
    error       TEXT
);

CREATE TABLE coming_soon_superchargers (
    uuid              TEXT PRIMARY KEY,
    title             TEXT NOT NULL,
    latitude          DOUBLE PRECISION NOT NULL,
    longitude         DOUBLE PRECISION NOT NULL,
    status            site_status NOT NULL DEFAULT 'UNKNOWN',
    location_url_slug TEXT,
    raw_status_value  TEXT,
    first_seen_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_scraped_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    opened_at         TIMESTAMPTZ
);

CREATE TABLE status_changes (
    id                BIGSERIAL PRIMARY KEY,
    supercharger_uuid TEXT NOT NULL REFERENCES coming_soon_superchargers(uuid),
    scrape_run_id     BIGINT NOT NULL REFERENCES scrape_runs(id),
    old_status        site_status,         -- NULL = first time we see this charger
    new_status        site_status NOT NULL,
    changed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ON status_changes (supercharger_uuid);
CREATE INDEX ON coming_soon_superchargers (status);
```

`status_changes` covers three events:
- `old_status IS NULL` → newly discovered location
- `old_status != new_status` → status transition (e.g. in_development → under_construction)
- row in `coming_soon_superchargers` with `opened_at` set → charger went live (logged separately, see Step 3)

---

## Step 1 — Add `sqlx` and migration tooling

Add dependencies to `Cargo.toml`:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-native-tls", "postgres", "chrono", "migrate"] }
dotenvy = "0.15"
```

`dotenvy` loads `.env` at startup. Call `dotenvy::dotenv().ok()` at the top of `main` before any env var is read — `.ok()` so it silently does nothing if `.env` doesn't exist (e.g. in CI where vars are injected directly).

Create the migrations directory and initial migration file:

```
cargo install sqlx-cli --no-default-features --features native-tls,postgres
sqlx migrate add init
```

Paste the schema SQL above into the generated `migrations/<timestamp>_init.sql` file.

Migrations run automatically at startup (see Step 2) — no manual `sqlx migrate run` needed. The `sqlx-cli` is only needed locally to generate new migration files.

---

## Step 2 — Add a `db` module (`src/db.rs`)

Thin data-access layer — no business logic, just SQL. Four functions:

**`connect(database_url: &str) -> PgPool`**
Connects and immediately runs any pending migrations via `sqlx::migrate!()`. The macro embeds the `migrations/` directory into the binary at compile time, so no external files are needed at runtime:

```rust
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPool::connect(database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
```

Migrations are idempotent — sqlx tracks applied migrations in a `_sqlx_migrations` table and skips ones already run.

**`record_scrape_run(pool, country, total_count, error) -> i64`**
Inserts a row into `scrape_runs`, returns the new `id`.

**`get_current_statuses(pool, uuids: &[&str]) -> HashMap<String, ComingSoonStatus>`**
`SELECT uuid, status FROM coming_soon_superchargers WHERE uuid = ANY($1)`.
Returns whatever is currently in the DB so the sync layer can diff against it.

**`save_chargers(pool, chargers, scrape_run_id)`**
Unconditional upsert — does not decide what to write, just writes what it's told:
- `INSERT ... ON CONFLICT (uuid) DO UPDATE` for each charger (title, coords, status, raw_status_value, last_scraped_at).
- Bulk-insert any `StatusChange` rows it receives.
- Bulk-set `opened_at = NOW()` for any UUIDs it's told have gone live.

No comparisons, no branching on old vs new — that's the sync layer's job.

---

## Step 3 — Add a `sync` module (`src/sync.rs`)

This is where the diffing logic lives. One function:

**`compute_sync(current: HashMap<String, ComingSoonStatus>, fresh: &[ComingSoonSupercharger]) -> SyncPlan`**

Compares what's in the DB (`current`) against the latest scrape (`fresh`) and returns a plain `SyncPlan` struct:

```rust
struct SyncPlan {
    upserts: Vec<ComingSoonSupercharger>,  // all fresh chargers (always upsert)
    status_changes: Vec<StatusChange>,     // new entries + transitions
    opened_uuids: Vec<String>,             // in DB but absent from fresh scrape
}

struct StatusChange {
    supercharger_uuid: String,
    old_status: Option<ComingSoonStatus>,  // None = first time seen
    new_status: ComingSoonStatus,
}
```

Rules:
- UUID not in `current` → `StatusChange { old_status: None, new_status: fresh.status }`
- UUID in `current` with different status → `StatusChange { old_status: Some(old), new_status: fresh.status }`
- UUID in `current` but not in `fresh` → add to `opened_uuids`

No DB calls, no side effects — pure logic, trivially testable.

---

## Step 4 — Wire it together in `main.rs`

`DATABASE_URL` is required — fail fast with a clear error if it's not set.

After building `coming_soon`:

```rust
dotenvy::dotenv().ok();
let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

let pool = db::connect(&database_url).await?;
let run_id = db::record_scrape_run(&pool, &args.country, coming_soon.len() as i32, None).await?;

let uuids: Vec<&str> = coming_soon.iter().map(|c| c.uuid.as_str()).collect();
let current = db::get_current_statuses(&pool, &uuids).await?;

let plan = sync::compute_sync(current, &coming_soon);
db::save_chargers(&pool, &plan, run_id).await?;

println!("Saved {} locations ({} status changes, {} opened)",
    plan.upserts.len(), plan.status_changes.len(), plan.opened_uuids.len());
```

---

## Step 5 — Useful queries (reference)

**All status transitions ever:**
```sql
SELECT c.title, sc.old_status, sc.new_status, sc.changed_at
FROM status_changes sc
JOIN coming_soon_superchargers c ON c.uuid = sc.supercharger_uuid
WHERE sc.old_status IS NOT NULL
ORDER BY sc.changed_at DESC;
```

**Locations that went from in_development → under_construction:**
```sql
SELECT c.title, sc.changed_at
FROM status_changes sc
JOIN coming_soon_superchargers c ON c.uuid = sc.supercharger_uuid
WHERE sc.old_status = 'IN_DEVELOPMENT'
  AND sc.new_status = 'UNDER_CONSTRUCTION'
ORDER BY sc.changed_at DESC;
```

**Currently active coming-soon locations by status:**
```sql
SELECT status, COUNT(*) FROM coming_soon_superchargers
WHERE opened_at IS NULL
GROUP BY status;
```
