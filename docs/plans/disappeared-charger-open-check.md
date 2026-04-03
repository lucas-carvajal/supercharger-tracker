# Plan: Detect opened/removed disappeared chargers & drop is_active

## Context

When a charger disappears from the coming-soon scrape it currently gets marked
`is_active = FALSE` with no explanation. By hitting the Tesla detail endpoint with
`functionTypes=supercharger` for each disappeared ID we can distinguish two cases:

- **Opened** → `data.supercharger_function.site_status == "open"` → status set to `OPEN`
- **Removed/unknown** → `data == {}` (HTTP 200, empty object) → status set to `REMOVED` + warning log

Once `OPEN` and `REMOVED` are terminal statuses, `is_active` is fully redundant —
"active" chargers are those whose status is not `OPEN` or `REMOVED` — so the column is dropped.

**Why `OPEN` must be added to the `site_status` enum:** `status_changes.new_status` is
typed `site_status NOT NULL`, so inserting an Open status-change row requires the value
to exist in the enum.

**Chrome verification (slug 30138, opened 2026-04-01):**
```
data.supercharger_function.site_status        = "open"
data.functions[0].opening_date                = "2026-04-01"
data.supercharger_function.num_charger_stalls = "16"
data.supercharger_function.open_to_non_tesla  = true
```
Non-existent/removed slugs return `{ "data": {} }`.

---

## Scope: modes removed

Only browser mode is kept. The following are deleted entirely:

| Removed | Reason |
|---|---|
| `--file PATH` scrape flag | Dev/replay shortcut, not needed in prod |
| `--cookie STRING` scrape flag | Browser handles auth automatically |
| `--cookie STRING` retry-failed flag | Same |
| `load_from_file()` | File mode |
| `load_with_cookie()` | Cookie mode |
| `fetch_details_only_cookie()` | Cookie-based detail fetch |
| `build_cookie_client()` | Cookie HTTP client helper |

`load_from_browser()`, `fetch_details_only_browser()`, and `launch_browser_and_wait()`
are kept unchanged. `--show-browser` flag stays on both `scrape` and `retry-failed`.

---

## Migrations

### `20260403000001_add_open_removed_status.sql`
```sql
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'OPEN';
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'REMOVED';
```

### `20260403000002_drop_is_active.sql`
```sql
ALTER TABLE coming_soon_superchargers DROP COLUMN is_active;
DROP INDEX IF EXISTS coming_soon_superchargers_is_active_idx;
```

### `20260403000003_create_opened_superchargers.sql`
```sql
CREATE TABLE opened_superchargers (
  id                TEXT PRIMARY KEY,
  opening_date      DATE,
  num_stalls        INTEGER,
  open_to_non_tesla BOOLEAN,
  detected_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

`title`, `city`, `region` are intentionally omitted — they already live on the
`coming_soon_superchargers` record (which is kept with `is_active` replaced by
`status = 'OPEN'`) and are accessible via join on `id`.

---

## Code changes

### `src/coming_soon.rs`
- Add `Open` and `Removed` to `SiteStatus`.
- `Display`: `Open` → `"Open"`, `Removed` → `"Removed"`.

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
    pub num_charger_stalls: Option<String>,   // comes as a string from the API
    pub open_to_non_tesla: Option<bool>,
}
pub struct OpenCheckFunction {
    pub opening_date: Option<String>,  // "YYYY-MM-DD"
}
```

### `src/loaders.rs`
- **Delete**: `load_from_file`, `load_with_cookie`, `fetch_details_only_cookie`, `build_cookie_client`.
- **Add**: `fetch_open_status_for_ids` (browser-only, no cookie parameter):

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

Uses `launch_browser_and_wait` + JS eval against the `functionTypes=supercharger` endpoint.

### `src/sync.rs`
Change `disappeared_ids: Vec<String>` → `Vec<(String, SiteStatus)>` to carry the
old status for building `StatusChange` records.

```rust
let disappeared_ids = current
    .into_iter()
    .filter(|(id, _)| !fresh_ids.contains(id.as_str()))
    .collect();
```

Update the `absent_from_scrape_goes_to_disappeared` test to use tuple form.

### `src/db.rs`
- **`get_current_statuses`**, **`get_current_db_stats`**, **`get_failed_detail_chargers`**,
  **`list_coming_soon`**, **`count_coming_soon_by_status`**, **`list_recent_additions`**:
  replace all `WHERE is_active = TRUE/true` with `WHERE status NOT IN ('OPEN', 'REMOVED')`.

- **`ApiSupercharger`**: remove `is_active: bool` field and all its SELECT/mapping references.

- **`save_chargers`**:
  - Signature: `disappeared_ids: &[(String, SiteStatus)]`.
  - INSERT / ON CONFLICT: remove `is_active` column and `is_active = TRUE`.
  - Replace the single `UPDATE SET is_active = FALSE` with two targeted updates
    (one sets `status = 'OPEN'` for opened IDs, one sets `status = 'REMOVED'` for
    removed IDs); the caller supplies two separate ID lists.

- **New**: `insert_opened_supercharger(pool, id, OpenResult)`.

### `src/api/superchargers.rs`
Remove `is_active` from `DetailResponse` and its population.

### `src/main.rs`

**`run_scrape`** — remove `file` and `cookie` params, always call `load_from_browser`.
After `compute_sync`:

```rust
let open_results: HashMap<String, OpenResult> =
    if plan.disappeared_ids.is_empty() { HashMap::new() }
    else {
        let ids: Vec<String> = plan.disappeared_ids.iter().map(|(id,_)| id.clone()).collect();
        loaders::fetch_open_status_for_ids(&ids, show_browser).await.unwrap_or_default()
    };

let (mut opened_ids, mut removed_ids) = (vec![], vec![]);
let mut extra_status_changes = vec![];

for (id, old_status) in &plan.disappeared_ids {
    if open_results.contains_key(id) {
        extra_status_changes.push(StatusChange { supercharger_id: id.clone(),
            old_status: Some(old_status.clone()), new_status: SiteStatus::Open });
        db::insert_opened_supercharger(pool, id, &open_results[id]).await?;
        opened_ids.push(id.clone());
    } else {
        eprintln!("  ⚠ Disappeared charger {id} not found in Tesla API — may have been removed");
        extra_status_changes.push(StatusChange { supercharger_id: id.clone(),
            old_status: Some(old_status.clone()), new_status: SiteStatus::Removed });
        removed_ids.push(id.clone());
    }
}
```

**`run_retry_failed`** — remove `cookie` param, always use browser.

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
- `migrations/20260403000001_add_open_removed_status.sql`
- `migrations/20260403000002_drop_is_active.sql`
- `migrations/20260403000003_create_opened_superchargers.sql`

---

## Verification
1. `cargo build`
2. `cargo test` — update `absent_from_scrape_goes_to_disappeared` for tuple form.
3. Manual: `cargo run -- scrape` (browser mode only):
   - Opened → row in `opened_superchargers` with opening_date/num_stalls/open_to_non_tesla,
     `status = 'OPEN'` in `coming_soon_superchargers`, status_change with `new_status = 'OPEN'`
   - Empty API → warning in stderr, `status = 'REMOVED'`, status_change with `new_status = 'REMOVED'`
   - No `is_active` column on the table.
   - `--file` and `--cookie` flags no longer accepted.
