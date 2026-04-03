# Plan: Detect opened/removed disappeared chargers & drop is_active

## Context

When a charger disappears from the coming-soon scrape it currently gets marked
`is_active = FALSE` with no explanation. By hitting the Tesla detail endpoint with
`functionTypes=supercharger` for each disappeared ID we can distinguish two cases:

- **Opened** → `data.supercharger_function.site_status == "open"` → DELETE from
  `coming_soon_superchargers`, INSERT into `opened_superchargers`
- **Removed/unknown** → `data == {}` (HTTP 200, empty object) → set `status = 'REMOVED'`
  in `coming_soon_superchargers` + warning log

Because opened chargers are deleted rather than status-updated, **`OPEN` does not need
to be added to the `site_status` enum** — only `REMOVED` is added.

Once `REMOVED` is the only terminal status, `is_active` is redundant:
"active" = `WHERE status != 'REMOVED'`. The column is dropped.

`status_changes.supercharger_id` currently has a hard FK to `coming_soon_superchargers`.
Since we DELETE opened charger rows, the FK is **dropped** (column stays `NOT NULL`,
just no `REFERENCES` clause). Status-change rows become soft-references — they stay
in the table after the charger is deleted and remain fully queryable by id:
```sql
SELECT * FROM status_changes WHERE supercharger_id = '30138' ORDER BY changed_at ASC;
```
The index on `supercharger_id` is already in place so this is fast. The only trade-off
is no DB-level guard against a typo'd id in `status_changes`, but since the app is the
sole writer the risk is negligible. History for REMOVED chargers is unaffected — those
rows stay in `coming_soon_superchargers` so the FK drop only matters for opened ones.

**Chrome verification (slug 30138, opened 2026-04-01):**
```
data.supercharger_function.site_status        = "open"
data.functions[0].opening_date                = "2026-04-01"
data.supercharger_function.num_charger_stalls = "16"
data.supercharger_function.open_to_non_tesla  = true
```
Non-existent/removed slugs return `{ "data": {} }`.

---

## Migrations

### `20260403000001_add_removed_status.sql`
```sql
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'REMOVED';
```

### `20260403000002_drop_is_active.sql`
```sql
ALTER TABLE coming_soon_superchargers DROP COLUMN is_active;
DROP INDEX IF EXISTS coming_soon_superchargers_is_active_idx;
```

### `20260403000003_status_changes_drop_fk.sql`
```sql
-- Drop the FK so status_changes rows survive when an opened charger is deleted
-- from coming_soon_superchargers. Rows become soft-references, still queryable by id.
ALTER TABLE status_changes DROP CONSTRAINT status_changes_supercharger_id_fkey;
```

### `20260403000004_create_opened_superchargers.sql`
```sql
CREATE TABLE opened_superchargers (
  id                TEXT PRIMARY KEY,
  title             TEXT NOT NULL,
  city              TEXT,
  region            TEXT,
  latitude          DOUBLE PRECISION NOT NULL,
  longitude         DOUBLE PRECISION NOT NULL,
  first_seen_at     TIMESTAMPTZ NOT NULL,   -- copied from coming_soon_superchargers
  opening_date      DATE,
  num_stalls        INTEGER,
  open_to_non_tesla BOOLEAN,
  detected_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## Code changes

### `src/coming_soon.rs`
- Add `Removed` to `SiteStatus`.
- `Display`: `Removed` → `"Removed"`.

### `src/raw.rs`
New types for the `functionTypes=supercharger` response:
```rust
pub struct OpenCheckResponse { pub data: OpenCheckData }
pub struct OpenCheckData {
    pub supercharger_function: Option<OpenCheckSuperchargerFunction>,
    pub functions: Option<Vec<OpenCheckFunction>>,
}
pub struct OpenCheckSuperchargerFunction {
    pub site_status: Option<String>,
    pub num_charger_stalls: Option<String>,  // string in the API
    pub open_to_non_tesla: Option<bool>,
}
pub struct OpenCheckFunction {
    pub opening_date: Option<String>,  // "YYYY-MM-DD"
}
```

### `src/loaders.rs`
New public function (browser-only — cookie/file modes are removed in a separate PR):

```rust
pub struct OpenResult {
    pub opening_date: Option<NaiveDate>,
    pub num_stalls: Option<i32>,
    pub open_to_non_tesla: Option<bool>,
}

/// Returns a map of id → OpenResult for confirmed-opened chargers.
/// IDs absent from the map returned empty data (presumed removed).
pub async fn fetch_open_status_for_ids(
    ids: &[String],
    show_browser: bool,
) -> Result<HashMap<String, OpenResult>, Box<dyn std::error::Error>>
```

### `src/sync.rs`
Change `disappeared_ids: Vec<String>` → `Vec<(String, SiteStatus)>` to carry the
old status for building `StatusChange` records for removed chargers.

```rust
let disappeared_ids = current
    .into_iter()
    .filter(|(id, _)| !fresh_ids.contains(id.as_str()))
    .collect();
```

Update the `absent_from_scrape_goes_to_disappeared` test for the tuple form.

### `src/db.rs`
- **All `WHERE is_active = TRUE/true`** → `WHERE status != 'REMOVED'`.
- **`ApiSupercharger`**: remove `is_active` field from struct, SELECT, and mapping.
- **`save_chargers`** signature: `disappeared_ids: &[(String, SiteStatus)]`.
  - Remove `is_active` from INSERT / ON CONFLICT.
  - Replace `UPDATE SET is_active = FALSE` with `UPDATE SET status = 'REMOVED'`
    for the removed IDs (opened IDs are handled separately via DELETE).
- **New** `delete_coming_soon(tx, id)` — DELETE from `coming_soon_superchargers`.
  With the FK dropped, `status_changes` rows survive and remain queryable by id.
- **New** `insert_opened_supercharger(tx, id, &ComingSoonSupercharger, &OpenResult)` —
  INSERT into `opened_superchargers` (copies title/city/region/lat/lon/first_seen_at
  from the charger record before it is deleted).

Both functions take `&mut Transaction` and are called within the **same transaction as
the rest of `save_chargers`**: upserts, status_changes inserts, the DELETE, and the
`opened_superchargers` INSERT all commit or roll back together. If the INSERT fails,
the DELETE is rolled back — the charger stays in `coming_soon_superchargers` and no
orphaned `status_changes` rows are produced.

### `src/api/superchargers.rs`
Remove `is_active` from `DetailResponse` and its population.

### `src/main.rs` — `run_scrape`
After `compute_sync`, fetch open status and split disappeared IDs:

```rust
let open_results = if plan.disappeared_ids.is_empty() { HashMap::new() } else {
    let ids = plan.disappeared_ids.iter().map(|(id,_)| id.clone()).collect::<Vec<_>>();
    loaders::fetch_open_status_for_ids(&ids, show_browser).await.unwrap_or_default()
};

let mut removed_ids: Vec<String> = vec![];
let mut removed_status_changes: Vec<StatusChange> = vec![];

for (id, old_status) in &plan.disappeared_ids {
    if open_results.contains_key(id) {
        // opened — handled inside save_chargers via delete + insert
    } else {
        eprintln!("  ⚠ Disappeared charger {id} not found in Tesla API — may have been removed");
        removed_ids.push(id.clone());
        removed_status_changes.push(StatusChange {
            supercharger_id: id.clone(),
            old_status: Some(old_status.clone()),
            new_status: SiteStatus::Removed,
        });
    }
}

let mut all_status_changes = plan.status_changes;
all_status_changes.extend(removed_status_changes);
```

Pass `open_results`, `removed_ids`, and `all_status_changes` to `save_chargers`.

Update summary print to show opened/removed counts.

### `docs/API.md`
Remove `is_active` from response examples and field docs.

---

## Critical files
- `src/coming_soon.rs`
- `src/raw.rs`
- `src/loaders.rs`
- `src/sync.rs`
- `src/db.rs`
- `src/main.rs`
- `src/api/superchargers.rs`
- `docs/API.md`
- `migrations/20260403000001_add_removed_status.sql`
- `migrations/20260403000002_drop_is_active.sql`
- `migrations/20260403000003_status_changes_drop_fk.sql`
- `migrations/20260403000004_create_opened_superchargers.sql`

---

## Verification
1. `cargo build`
2. `cargo test` — update `absent_from_scrape_goes_to_disappeared` for tuple form.
3. Manual: `cargo run -- scrape`:
   - Opened → row in `opened_superchargers` with all fields, row DELETED from
     `coming_soon_superchargers`, status_changes rows **preserved** (soft-reference by id)
   - Removed → `status = 'REMOVED'` in `coming_soon_superchargers`, warning in stderr,
     status_change row with `new_status = 'REMOVED'`
   - `is_active` column no longer present.
