# Plan: Check if disappeared chargers have opened or been removed

## Context

When a charger disappears from the coming-soon scrape it currently just gets marked `is_active = FALSE` with no explanation. By hitting the Tesla detail endpoint with `functionTypes=supercharger` for each disappeared ID we can distinguish two cases:

- **Opened** → `data.supercharger_function.site_status == "open"` — confirmed graduated to a live Supercharger
- **Removed/unknown** → `data == {}` (HTTP 200, empty object) — not in Tesla's system anymore

**Chrome verification:**
- Opened charger (30138, 2026-04-01): `data.supercharger_function.site_status: "open"`, `data.functions[0].opening_date: "2026-04-01"`
- Non-existent/removed (slug 99999, 500): `{ "data": {} }` — no `supercharger_function`

**Design:**
- Add both `OPEN` and `REMOVED` to `site_status` enum — every lifecycle transition is recorded in `status_changes`
- `coming_soon_superchargers` record is kept for all cases; status updated to `OPEN`/`REMOVED`, then `is_active = FALSE`
- An `opened_superchargers` table stores the opening date from the API for confirmed-opened chargers

---

## Changes

### 1. Migrations
**`migrations/20260403000001_add_open_removed_status.sql`**
```sql
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'OPEN';
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'REMOVED';
```

**`migrations/20260403000002_create_opened_superchargers.sql`**
```sql
CREATE TABLE opened_superchargers (
  id            TEXT PRIMARY KEY,
  opening_date  DATE,
  detected_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 2. `src/coming_soon.rs`
- Add `Open` and `Removed` variants to `SiteStatus` enum.
- Update `Display`: `Open` → `"Open"`, `Removed` → `"Removed"`.

### 3. `src/raw.rs`
Add types to deserialize the `functionTypes=supercharger` response:

```rust
#[derive(Deserialize)]
pub struct OpenCheckResponse {
    pub data: OpenCheckData,
}

#[derive(Deserialize)]
pub struct OpenCheckData {
    pub supercharger_function: Option<OpenCheckSuperchargerFunction>,
    pub functions: Option<Vec<OpenCheckFunction>>,
}

#[derive(Deserialize)]
pub struct OpenCheckSuperchargerFunction {
    pub site_status: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenCheckFunction {
    pub opening_date: Option<String>,  // "YYYY-MM-DD"
}
```

### 4. `src/loaders.rs` — new public function
```rust
/// For a set of disappeared charger IDs, check if they've opened.
/// Returns a map of id → opening_date (None means opened but no date available).
/// IDs absent from the map had an empty API response (presumed removed).
pub async fn fetch_open_status_for_ids(
    ids: &[String],
    cookie: Option<&str>,
    show_browser: bool,
) -> Result<HashMap<String, Option<NaiveDate>>, Box<dyn std::error::Error>>
```
- Returns empty map immediately if `ids` is empty.
- URL: `{DETAILS_URL}?locationSlug={id}&functionTypes=supercharger&locale=en_US&isInHkMoTw=false`
- **Cookie path:** concurrent reqwest fetch, same concurrency as `fetch_details_with_client`.
- **Browser path:** `launch_browser_and_wait` + JS eval (same pattern as `eval_detail_batch`).
- Per response: if `site_status == "open"` → insert into map with `opening_date` parsed from `functions[0].opening_date`; otherwise (empty data) → absent from map.

### 5. `src/sync.rs`
Change `disappeared_ids: Vec<String>` → `Vec<(String, SiteStatus)>` in `SyncPlan` to carry the old DB status alongside each disappeared ID (needed to construct `StatusChange` records for Open/Removed transitions).

Update collection in `compute_sync`:
```rust
let disappeared_ids = current
    .into_iter()
    .filter(|(id, _)| !fresh_ids.contains(id.as_str()))
    .collect();
```

Update the existing `absent_from_scrape_goes_to_disappeared` test to use the tuple form.

### 6. `src/db.rs`
- **`save_chargers`**: update `disappeared_ids` parameter to `&[(String, SiteStatus)]`; extract IDs for the `is_active = FALSE` deactivation query.
- **New function** `insert_opened_supercharger(pool, id: &str, opening_date: Option<NaiveDate>)` — `INSERT INTO opened_superchargers`.

### 7. `src/main.rs` — `run_scrape`
After `compute_sync`, before `save_chargers`:

1. Fetch open status for disappeared IDs (skip in file mode — no auth).
2. For **opened**: push `StatusChange { new_status: Open }`, call `insert_opened_supercharger`.
3. For **empty API** (removed): push `StatusChange { new_status: Removed }`, emit `eprintln!` warning.
4. Merge extra status changes into `plan.status_changes`.
5. Deactivation (`is_active = FALSE`) still happens via `save_chargers` for all disappeared IDs.
6. Update summary print to include opened/removed counts.

**File mode:** check skipped; disappeared chargers deactivated silently as before.
**Browser mode:** launches a second browser session only when `disappeared_ids` is non-empty (~8s extra; uncommon).

---

## Verification
1. `cargo build`
2. `cargo test`
3. Manual run with `--cookie`: confirm opened → row in `opened_superchargers` + `status = 'OPEN'` + status_change; removed → warning + `status = 'REMOVED'` + status_change.
