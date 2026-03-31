# Plan: Region Filtering for Supercharger API

## Goal

Allow `GET /superchargers/soon` to be filtered by region — either a country name,
a US state abbreviation, or an AU state/territory abbreviation:

```
GET /superchargers/soon?region=Denmark
GET /superchargers/soon?region=US        # expands to all US states
GET /superchargers/soon?region=CA        # California specifically
GET /superchargers/soon?region=AU        # expands to all AU states/territories
GET /superchargers/soon?region=NSW       # New South Wales specifically
```

Invalid values return `400 Bad Request`.

---

## Background

Charger `title` values follow the pattern `"City, Region"`:

- `"Copenhagen, Denmark"` — international
- `"West Sacramento, CA"` — US state abbreviation
- `"Sydney, NSW"` — Australian state abbreviation
- `"location"` — malformed; no useful location data, skip

The region portion (everything after the last comma, trimmed) is what is stored
and filtered on. No backfill of existing rows — they populate on the next scrape.

---

## DB Changes

### New migration

Add nullable `city` and `region` columns to `coming_soon_superchargers`, with an
index on `region` to back the filter query:

```sql
ALTER TABLE coming_soon_superchargers
    ADD COLUMN city   TEXT,
    ADD COLUMN region TEXT;

CREATE INDEX ON coming_soon_superchargers (region);
```

---

## Parsing (`coming_soon.rs`)

Add a `parse_title(title: &str) -> (Option<String>, Option<String>)` helper:

- Split on the **last comma**
- Trim both sides
- If no comma, or either side is empty → `(None, None)` (handles `"location"` etc.)

Call it from `ComingSoonSupercharger::from_location` and store results in new
`city: Option<String>` and `region: Option<String>` fields on the struct.

---

## Allowlist & Mapping (`api/superchargers.rs` or a dedicated `regions.rs`)

Define a static mapping that resolves an API `?region=` value to one or more DB
`region` strings. Invalid input → `400`.

```
"US"  → ["AL","AK","AZ","AR","CA","CO","CT","DE","FL","GA",
          "HI","ID","IL","IN","IA","KS","KY","LA","ME","MD",
          "MA","MI","MN","MS","MO","MT","NE","NV","NH","NJ",
          "NM","NY","NC","ND","OH","OK","OR","PA","RI","SC",
          "SD","TN","TX","UT","VT","VA","WA","WV","WI","WY","DC"]

"AU"  → ["NSW","VIC","QLD","SA","WA","TAS","NT","ACT"]

Individual state/territory abbreviations → ["CA"], ["NSW"], etc.
(treated as direct pass-through after validating membership in the allowlist)

All other entries → single-element list, e.g. "Denmark" → ["Denmark"]
Countries: hardcoded list of all countries where Tesla operates.
```

**Note on hardcoded country names:** Before finalising the list, query actual
Tesla data to get the ground-truth spellings:

```sql
SELECT DISTINCT region, COUNT(*) AS cnt
FROM coming_soon_superchargers
WHERE region IS NOT NULL
ORDER BY cnt DESC;
```

Run this after the first scrape post-migration. Tesla titles use full country
names (e.g. `"United Kingdom"`, `"Germany"`) — spellings may differ from
expectations (e.g. `"Czechia"` not `"Czech Republic"`).

The resolved list is passed to the DB layer as `Vec<String>`.

**Unknown region logging:** When a `?region=` value is not found in the
allowlist, log it to stderr before returning `400`:

```
eprintln!("[region-filter] unknown region requested: {value:?}");
```

This lets us monitor for regions that should be added to the list (e.g. a new
Tesla market, or an unexpected spelling variant) without needing a full logging
framework.

---

## DB Query

`list_coming_soon` gains a `region_filter: Option<&[String]>` parameter.
When set, the WHERE clause becomes:

```sql
WHERE is_active = true
  AND region = ANY($N::text[])
```

`= ANY(array)` with the `region` index is efficient:
- Single country → index equality lookup
- US/AU expansion (50+ values) → bitmap index scan; fast at any realistic table size

When `region_filter` is `None` (no `?region=` param), the clause is omitted entirely.

---

## API Response

Add `city` and `region` to all response types that include charger detail:
`SuperchargerItem`, `DetailResponse`, `RecentAdditionItem`.

Both fields are nullable (`null` in JSON when the title couldn't be parsed).

---

## Files Touched

| File | Change |
|---|---|
| `migrations/20260401000000_location_columns.sql` | New migration |
| `src/coming_soon.rs` | Add `city`/`region` fields + `parse_title` fn |
| `src/db.rs` | Add fields to `ApiSupercharger`, update all SELECT queries, update `save_chargers` upsert, add `region_filter` param to `list_coming_soon` |
| `src/api/superchargers.rs` | Add `?region=` query param, allowlist mapping, pass filter to DB, add fields to response types |
| `src/sync.rs` | Add `city: None, region: None` to test helper struct literals |
| `docs/API.md` | Document `?region=` param and new response fields |

---

## Out of Scope

- Backfilling existing rows (they populate on next scrape)
- A `GET /superchargers/soon/regions` discovery endpoint (nice-to-have later)
