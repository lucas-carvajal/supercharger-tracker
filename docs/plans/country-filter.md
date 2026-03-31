# Plan: Country/Region Filtering for Supercharger API

## Goal

Allow `GET /superchargers/soon` to be filtered by country or US state, e.g.:

```
GET /superchargers/soon?country=Denmark
GET /superchargers/soon?country=CA
```

---

## Background

Charger `title` values follow the pattern `"City, Region"`:

- `"Copenhagen, Denmark"` — international
- `"West Sacramento, CA"` — US state abbreviation
- `"location"` — malformed, no useful location data

The `region` portion (after the last comma) is what you'd filter on. There is no existing `country` column on `coming_soon_superchargers`; country is only tracked at the `scrape_runs` level and only represents what was queried, not where each charger is.

---

## Proposed Approach

### 1. New migration

Add `city` and `region` as nullable `TEXT` columns to `coming_soon_superchargers`, with an index on `region` to support fast filtering:

```sql
ALTER TABLE coming_soon_superchargers
    ADD COLUMN city   TEXT,
    ADD COLUMN region TEXT;

CREATE INDEX ON coming_soon_superchargers (region);
```

### 2. Parsing (`coming_soon.rs`)

Add a `parse_title(title: &str) -> (Option<String>, Option<String>)` function:
- Split on the **last comma** in the title
- Trim both sides
- If either side is empty or there is no comma → return `(None, None)` (handles `"location"` and similar garbage)
- Populate `city`/`region` fields on `ComingSoonSupercharger` in `from_location`

### 3. DB layer (`db.rs`)

- Add `city`/`region` to `ApiSupercharger` struct
- Update all `SELECT` queries to include `city, region`
- Update `save_chargers` bulk upsert to write `city`/`region`
- Add `country_filter: Option<&str>` parameter to `list_coming_soon`; filter with `region ILIKE $N` when set (case-insensitive, so `denmark` matches `Denmark`)

### 4. API handler (`api/superchargers.rs`)

- Add `country: Option<String>` to `ListQuery`
- Pass it through to `db::list_coming_soon`
- Add `city`/`region` fields to `SuperchargerItem`, `DetailResponse`, `RecentAdditionItem` response types

### 5. Docs (`docs/API.md`)

Document the new `country` query param and the new `city`/`region` response fields.

### 6. Test helpers (`sync.rs`)

Update the `charger()` and `charger_with_slug()` test helper structs to include `city: None, region: None` (no logic change, just struct completeness).

---

## Files Touched

| File | Change |
|---|---|
| `migrations/20260401000000_location_columns.sql` | New migration |
| `src/coming_soon.rs` | Add fields + `parse_title` fn |
| `src/db.rs` | Struct, queries, filter param |
| `src/api/superchargers.rs` | Query param, response types |
| `src/sync.rs` | Test helper struct literals |
| `docs/API.md` | Document new param and fields |

---

## Decisions / Tradeoffs to Discuss

### A. Column name: `country` vs `region`

The field after the last comma is sometimes a country (`"Denmark"`) and sometimes a US state abbreviation (`"CA"`). Naming it `country` in the DB would be misleading for US entries. Options:

- **`region`** (proposed) — neutral, accurate for both cases. API query param could still be called `?country=` for user-friendliness, mapping to the `region` column internally.
- **`country`** — simpler but technically wrong for US state abbreviations.

### B. Query param name: `?country=` vs `?region=`

Independent of the column name. `?country=CA` works naturally for international users. `?region=CA` is more technically accurate. Which do you prefer in the API surface?

### C. Exact match vs `ILIKE`

- **`ILIKE`** (proposed) — case-insensitive, so `?country=denmark` matches `"Denmark"`. Simpler for API consumers.
- **Exact match** — stricter, slightly faster, but requires the caller to know the exact casing (e.g. `"CA"` not `"ca"`).

### D. Backfilling existing rows

The migration adds the columns as `NULL`. Existing rows in the DB will have `NULL` city/region until the next scrape populates them. Options:

- **Do nothing** — they fill in naturally on next scrape. Fine if a scrape runs soon.
- **Backfill in migration** — parse `title` in SQL using `split_part` and `trim`. Messier SQL but no data gap. Only practical if the title format is reliable enough.
- **One-off backfill command** — add a temporary `backfill` subcommand that reads all rows and re-parses their titles. More work but clean.

### E. `retry-failed` and `unchanged` chargers

`retry-failed` re-saves chargers read from the DB. Those rows will have `city`/`region` from the DB (populated during their original scrape), so they're preserved correctly.

`unchanged` chargers (only `last_scraped_at` is touched) also keep their existing `city`/`region` — no change needed there.

---

## What I'd Recommend

- Use `region` as the DB column name, `?country=` as the API query param name (friendliest for callers)
- Use `ILIKE` for the filter
- Skip the backfill — wait for the next scrape (or run one manually after deploying)
