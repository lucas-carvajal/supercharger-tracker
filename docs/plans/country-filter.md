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

Invalid values return `400 Bad Request`. Matching is case-insensitive on the
input side (normalize in Rust before allowlist lookup, e.g. `"denmark"` → `"Denmark"`).

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

## Allowlist & Mapping (`src/regions.rs`)

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

**Canada:** Tesla may use province abbreviations (e.g. `"ON"`, `"BC"`) or full
province names for Canadian locations. This matters because `"CA"` is both the
California abbreviation and Canada's ISO code. Before implementing, check what
Tesla actually uses in their data — since the allowlist is derived from real DB
values, the correct approach will be clear from the scraped data. If Tesla uses
province abbreviations, a `"Canada"` aggregate key (mapping to all province
codes) avoids the conflict with `"CA"` = California.

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

`list_coming_soon` gains a `region_filter: &[String]` parameter (always present;
empty slice = no filter). The WHERE clause uses a `cardinality` check to avoid
needing four query branches (status × region):

```sql
WHERE is_active = true
  AND (status = $1::site_status)                               -- only when status filter active
  AND (cardinality($N::text[]) = 0 OR region = ANY($N::text[]))
```

`cardinality` of an empty Postgres array is `0`, so the condition short-circuits
to true when no region filter is passed. This keeps the existing two branches
(with/without status filter) without doubling them for region.

`= ANY(array)` with the `region` index is efficient:
- Single country → index equality lookup
- US/AU expansion (50+ values) → bitmap index scan; fast at any realistic table size

---

## API Response

Add `city` and `region` to all response types that include charger detail:
`SuperchargerItem`, `DetailResponse`, `RecentAdditionItem`, `RecentChangeItem`.

Both fields are nullable (`null` in JSON when the title couldn't be parsed).

---

## Files Touched

| File | Change |
|---|---|
| `migrations/20260401000000_location_columns.sql` | New migration |
| `src/coming_soon.rs` | Add `city`/`region` fields + `parse_title` fn |
| `src/db.rs` | Add fields to `ApiSupercharger`, update all SELECT queries, update `save_chargers` upsert, add `region_filter` param to `list_coming_soon` |
| `src/regions.rs` | New module: static allowlist + `resolve(input) -> Option<Vec<String>>` |
| `src/api/superchargers.rs` | Add `?region=` query param, call `regions::resolve`, pass filter to DB, add fields to response types |
| `src/sync.rs` | Add `city: None, region: None` to test helper struct literals |
| `src/main.rs` | Add `mod regions;` |
| `docs/API.md` | Document `?region=` param and new response fields |

---

## Verified Tesla Data (live API + DB, 2026-03-31)

This section records ground-truth findings from querying the live Tesla Find Us API
(`country=US` returns all worldwide coming-soon locations) and the existing DB.

### Title format

`"City, Region"` is consistent across all well-formed entries. `parse_title` splits
on the **last** comma and trims both sides — confirmed correct.

### Malformed titles (no comma → `(None, None)`)

All 4 distinct malformed forms confirmed from live data:

| Title | Notes |
|---|---|
| `locations` | 264 entries — Tesla data quality issue |
| `Ile Rousse` | City in Corsica with no region |
| `Tammisaari` | Finnish city with no region |
| `Avenches - Milavy Centre` | Swiss location with no region |

### Typos in Tesla data — NOT accommodated

These will parse correctly (city + region columns populated), but we do not add
allowlist entries for the misspelled region name. A `?region=Switerland` query
returns `400`. Users must filter by the correct country name.

| Raw title | Typo region | Real country |
|---|---|---|
| `Matran, Switerland` | `Switerland` | Switzerland |
| `Lomma, Swednen` | `Swednen` | Sweden |
| `Gainesville, Fl ` | `Fl` (+ trailing space) | Florida / US |

### Countries using state/province abbreviations

Four countries use sub-national codes rather than the country name:

| Country | Codes seen in data | Notes |
|---|---|---|
| **US** | CA TX FL MD WA VA NJ IL PA NY AZ OH WI CO NC OR GA MI DE MO MN SC IN KY NM NV WV WY UT IA CT DC HI RI AL ND OK MT AR TN LA | Standard 2-letter state + DC |
| **Australia** | NSW QLD VIC NT | Standard AU state/territory codes |
| **Canada** | BC ON AB NS MB QC SK | Standard province codes |
| **Mexico** | BCS COAH | Mexican state codes — but see dual-naming below |

### Dual-naming inconsistencies

Tesla uses two different region values for the same country in some entries.
The `resolve()` function must map user input to **all variants** so no entries are missed.

| Country | Variant A | Variant B | Example titles |
|---|---|---|---|
| New Zealand | `New Zealand` | `NZ` | `"Whakatāne, New Zealand"` / `"Auckland South, NZ"` |
| United Kingdom | `United Kingdom` | `UK` | `"Highbridge, United Kingdom"` / `"Strabane, UK"` |
| UAE | `United Arab Emirates` | `UAE`, `UAE - Dubai Silicon Oasis` | `"Abu Dhabi, UAE"` / `"Dubai, UAE - Dubai Silicon Oasis"` |
| Turkey | `Türkiye` | `Turkiye` | `"Ayazağa İstanbul, Türkiye"` / `"Afyonkarahisar, Turkiye"` |
| Mexico | `Mexico` | `BCS`, `COAH` (+ other state codes) | `"San Pedro Garza García, Mexico"` / `"La Paz, BCS"` |

### `resolve()` mappings

```
// Aggregate keys
"US"      → all US state abbreviations + "DC"
"AU"      → ["NSW","VIC","QLD","SA","WA","TAS","NT","ACT"]
"Canada"  → ["BC","ON","AB","SK","MB","QC","NB","NS","PE","NL","NT","YT","NU"]
"Mexico"  → ["Mexico","AGU","BCN","BCS","CAM","CHP","CHH","COA","COAH","COL",
              "CMX","DUR","GUA","GRO","HID","JAL","MEX","MIC","MOR","NAY",
              "NLE","OAX","PUE","QUE","ROO","SLP","SIN","SON","TAB","TAM",
              "TLX","VER","YUC","ZAC"]

// Multi-variant country mappings (user input → all DB spellings)
"United Kingdom" / "UK"              → ["United Kingdom", "UK"]
"Turkey" / "Turkiye" / "Türkiye"     → ["Türkiye", "Turkiye"]
"UAE" / "United Arab Emirates"       → ["United Arab Emirates", "UAE", "UAE - Dubai Silicon Oasis"]
"New Zealand" / "NZ"                 → ["New Zealand", "NZ"]

// Single-variant countries (one accepted input → one DB value)
// Exact DB spellings to use:
Germany, France, Spain, Norway, Sweden, Italy, Finland, Denmark, Hungary, Romania,
Czech Republic, Iceland, Ireland, Portugal, Croatia, Slovenia, Slovakia, Switzerland,
Austria, Netherlands, Poland, Latvia, Morocco, Taiwan, Thailand, Japan, South Korea,
Chile, Colombia, Israel, Saudi Arabia
```

**Note on `"Canada"` vs `"CA"`:** `"CA"` is unambiguously California in the DB — no
Canadian entries use `"CA"`. All Canadian locations use province abbreviations.

**Note on `"NT"`:** Appears in both Australian (Northern Territory) and Canadian
(Northwest Territories) data. Both are covered by their respective aggregate keys.

---

## Out of Scope

- Backfilling existing rows (they populate on next scrape)
- A `GET /superchargers/soon/regions` discovery endpoint (nice-to-have later)
- `?region=` filter on `recent-additions` and `recent-changes` (no schema changes
  required when this is added later — both queries already touch
  `coming_soon_superchargers` where `region` lives; purely a query + handler change)
