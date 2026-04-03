# Plan: Detect opened/removed disappeared chargers & drop is_active

## Context

When a charger disappears from the coming-soon scrape it currently gets marked
`is_active = FALSE` with no explanation. By hitting the Tesla detail endpoint with
`functionTypes=supercharger` for each disappeared ID we can distinguish two cases:

- **Opened** → `data.supercharger_function.site_status == "open"` — status → `OPEN`
- **Removed/unknown** → `data == {}` (HTTP 200, empty object) — status → `REMOVED` + warning log

Once `OPEN` and `REMOVED` are terminal statuses, `is_active` is fully redundant:
"active" chargers are simply those whose status is not `OPEN` or `REMOVED`.
Every disappearance is already explained by the status column, so the boolean can be dropped.

**Chrome verification:**
- Opened charger 30138 (2026-04-01): `data.supercharger_function.site_status: "open"`, `data.functions[0].opening_date: "2026-04-01"`
- Non-existent slug (99999, 500): `{ "data": {} }` — no `supercharger_function`

---

## Migrations

### `20260403000001_add_open_removed_status.sql`
```sql
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'OPEN';
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'REMOVED';
```

### `20260403000002_drop_is_active.sql`
```sql
-- All queries that filtered WHERE is_active = TRUE now filter
-- WHERE status NOT IN ('OPEN', 'REMOVED') instead.
ALTER TABLE coming_soon_superchargers DROP COLUMN is_active;
DROP INDEX IF EXISTS coming_soon_superchargers_is_active_idx;
```

### `20260403000003_create_opened_superchargers.sql`
```sql
CREATE TABLE opened_superchargers (
  id            TEXT PRIMARY KEY,
  opening_date  DATE,
  detected_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## Code changes

### `src/coming_soon.rs`
- Add `Open` and `Removed` variants to `SiteStatus`.
- Update `Display`: `Open` → `"Open"`, `Removed` → `"Removed"`.

### `src/raw.rs`
Add types to deserialize the `functionTypes=supercharger` detail response:
```rust
pub struct OpenCheckResponse { pub data: OpenCheckData }
pub struct OpenCheckData {
    pub supercharger_function: Option<OpenCheckSuperchargerFunction>,
    pub functions: Option<Vec<OpenCheckFunction>>,
}
pub struct OpenCheckSuperchargerFunction { pub site_status: Option<String> }
pub struct OpenCheckFunction { pub opening_date: Option<String> }  // "YYYY-MM-DD"
```

### `src/loaders.rs`
New public function:
```rust
pub async fn fetch_open_status_for_ids(
    ids: &[String],
    cookie: Option<&str>,
    show_browser: bool,
) -> Result<HashMap<String, Option<NaiveDate>>, Box<dyn std::error::Error>>
```
- Returns empty map if `ids` is empty.
- URL: `DETAILS_URL?locationSlug={id}&functionTypes=supercharger&locale=en_US&isInHkMoTw=false`
- Cookie path: concurrent reqwest, same pattern as `fetch_details_with_client`.
- Browser path: `launch_browser_and_wait` + JS eval.
- `site_status == "open"` → insert id into map with `opening_date` from `functions[0]`; otherwise absent from map.

### `src/sync.rs`
Change `disappeared_ids: Vec<String>` → `Vec<(String, SiteStatus)>` in `SyncPlan`
to carry the old status (needed to build `StatusChange` for Open/Removed transitions).

```rust
// compute_sync: collection becomes
let disappeared_ids = current
    .into_iter()
    .filter(|(id, _)| !fresh_ids.contains(id.as_str()))
    .collect();
```

Update the `absent_from_scrape_goes_to_disappeared` test to use the tuple form.

### `src/db.rs`

**`get_current_statuses`** — remove `is_active` filter:
```sql
SELECT id, status FROM coming_soon_superchargers
WHERE status NOT IN ('OPEN', 'REMOVED')
```

**`get_current_db_stats`** — remove `is_active` filter:
```sql
WHERE status NOT IN ('OPEN', 'REMOVED')
```

**`get_failed_detail_chargers`** — remove `is_active` filter:
```sql
WHERE status NOT IN ('OPEN', 'REMOVED') AND details_fetch_failed = TRUE
```

**`list_coming_soon`**, **`count_coming_soon_by_status`**, **`list_recent_additions`** —
replace all `WHERE is_active = true` with `WHERE status NOT IN ('OPEN', 'REMOVED')`.

**`ApiSupercharger`** — remove `is_active: bool` field.

**`get_coming_soon`** — remove `is_active` from SELECT and struct mapping.

**`save_chargers`** — three changes:
1. Signature: `disappeared_ids: &[(String, SiteStatus)]`, extract IDs for DB ops.
2. INSERT / ON CONFLICT: remove `is_active` column and `is_active = TRUE` update.
3. Replace the disappeared `UPDATE SET is_active = FALSE` with a status update:
   ```sql
   UPDATE coming_soon_superchargers
   SET status = new_status_per_id  -- OPEN or REMOVED, driven by open_results
   WHERE id = ANY($1)
   ```
   Since each ID gets a different terminal status, the simplest approach is to
   do two targeted updates (one for opened IDs, one for removed IDs) rather than
   a single bulk update.

**New function** `insert_opened_supercharger(pool, id, opening_date)`.

### `src/api/superchargers.rs`
Remove `is_active` from `DetailResponse` struct and its population.

### `src/main.rs` — `run_scrape`

After `compute_sync`, before `save_chargers`:
```rust
let open_results: HashMap<String, Option<NaiveDate>> =
    if !plan.disappeared_ids.is_empty() && file.is_none() {
        let ids: Vec<String> = plan.disappeared_ids.iter().map(|(id,_)| id.clone()).collect();
        loaders::fetch_open_status_for_ids(&ids, cookie.as_deref(), show_browser)
            .await.unwrap_or_default()
    } else { HashMap::new() };

let mut extra_status_changes = Vec::new();
for (id, old_status) in &plan.disappeared_ids {
    if let Some(opening_date) = open_results.get(id) {
        extra_status_changes.push(StatusChange { supercharger_id: id.clone(),
            old_status: Some(old_status.clone()), new_status: SiteStatus::Open });
        db::insert_opened_supercharger(pool, id, *opening_date).await?;
    } else if file.is_none() {
        eprintln!("  ⚠ Disappeared charger {id} not found in Tesla API — may have been removed");
        extra_status_changes.push(StatusChange { supercharger_id: id.clone(),
            old_status: Some(old_status.clone()), new_status: SiteStatus::Removed });
    }
}
let mut all_status_changes = plan.status_changes;
all_status_changes.extend(extra_status_changes);
```

Pass the open/removed breakdown to `save_chargers` so it can set the correct terminal
status on each disappeared row (instead of the old `is_active = FALSE` bulk update).

Update the summary print line to include opened/removed counts.

**File mode:** open-status check skipped; disappeared chargers still have their status
set to `REMOVED` is not possible without auth — for file mode we leave them without a
terminal status update (just remove them from the active set).
> **Open question:** For file mode, disappeared chargers currently just get `is_active=FALSE`.
> Without that column, we need to either: (a) set `REMOVED` anyway (even without confirming),
> or (b) leave them in their current status and accept that they'll show up in "active"
> queries until the next non-file scrape resolves them. Option (a) is simpler.

### `docs/API.md`
Remove `is_active` from the response example and field docs.

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
- `migrations/20260403000001_add_open_removed_status.sql`
- `migrations/20260403000002_drop_is_active.sql`
- `migrations/20260403000003_create_opened_superchargers.sql`

---

## Verification
1. `cargo build`
2. `cargo test` — update `absent_from_scrape_goes_to_disappeared` test for tuple form.
3. Manual: `cargo run -- scrape --cookie "..."` — confirm:
   - Opened → row in `opened_superchargers`, `status = 'OPEN'`, status_change row with `new_status = 'OPEN'`
   - Empty API → warning in stderr, `status = 'REMOVED'`, status_change row with `new_status = 'REMOVED'`
   - `is_active` column no longer present in the table.
