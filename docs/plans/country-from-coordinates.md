# Plan: Derive `country` from coordinates

## Goal

`region` is parsed from Tesla’s title (`"City, Region"`). That value is a mix of
country names (`United Kingdom`), state abbreviations (`TX`), spelling variants
(`UK` / `Türkiye`), and unparseable titles (`null`). It cannot support a reliable
“show me every charger in Germany” query.

Every coming-soon and opened row already has required `latitude` / `longitude`.
Add a separate, coordinate-derived `country` column so country search is
stable and independent of Tesla’s title.

## Recommended design (proposed defaults)

These are the defaults this plan would implement unless an open question
below overrides them.

1. **New nullable `TEXT` column `country`**, ISO 3166-1 alpha-2 (`DE`, `GB`,
   `US`). Not a full name. Not Tesla’s title fragment.
2. **Keep `region` and Tesla `country_code` as-is.** `region` stays the
   title-parsed display/filter field. `country_code` stays the optional Tesla
   details `address.country`. Do not reuse or replace either.
3. **Add the column to both** `coming_soon_superchargers` and
   `opened_superchargers`. Graduation copies `country` with the other fields.
4. **Derive with [`country-boundaries`](https://crates.io/crates/country-boundaries)**
   (offline, OSM polygons). Load `BOUNDARIES_ODBL_360X180` once as a process
   singleton.
5. **Pick the ISO-2 id** from `ids(lat, lng)` — the 2-letter code with no
   hyphen. Ignore subdivisions (`US-TX`, `GB-ENG`). If the point is not on
   land, store `NULL`.
6. **Recompute on every write** that has coordinates (scrape upsert, retry,
   import, snapshot restore, graduation). The lookup is cheap; coords can
   drift.
7. **New list filter `?country=DE`** on `GET /superchargers/soon`. Exact
   match, case-insensitive, ISO-2 only. `?region=` stays. Both can combine
   with `status`.
8. **Expose `country` on read API items** (list, detail, recent-*, map).
9. **Index** `coming_soon_superchargers (country)`.
10. **ODbL attribution** in README / docs (required by the default dataset).

`scrape_runs.country` is unrelated: it is the Tesla Find Us query country
(`scrape --country DE`; `US` means worldwide). Do not conflate it with the
new charger column.

## Why not reuse Tesla `country_code`

`country_code` is only set when the details fetch succeeds and Tesla returns
`address.country`. Details-failed rows have lat/lng but no Tesla country.
The new column must be computable from coordinates alone so a failed details
fetch still gets a searchable country.

Keeping both lets us compare Tesla vs derived later if we want.

## Derivation rule

```text
ids = country_boundaries.ids(LatLon { lat, lng })
iso2 = first 2-letter code in ids that does not contain '-'
        (ids are ordered smallest-area first; skip US-TX, take US)
if none → NULL
```

Known mappings to document in the API:

| Point | Derived `country` | Notes |
|---|---|---|
| Austin, TX | `US` | `ids` is `["US-TX", "US"]` |
| Highbridge, UK | `GB` | ISO is `GB`, not `UK` |
| Coastal / water | `NULL` | Dataset is “oblivious of sea borders” |
| Disputed / overlapping | TBD | See Q4 |

## Write paths that must set `country`

| Path | What to do |
|---|---|
| `ComingSoonSupercharger::from_location` / `with_details` | Derive from `latitude`/`longitude` |
| `SuperchargerRepository::save_chargers` upsert + unchanged update | Persist `country` (not gated on `details_fetch_failed`) |
| Graduation → `opened_superchargers` | Copy `country` (or re-derive from the same coords) |
| `export-diff` / `export-snapshot` | Include `country` on coming-soon and opened export types |
| `import` (diff + snapshot) | Prefer payload `country` when present; if missing (old files), derive from lat/lng so prod still gets the column |
| Existing DB rows | Backfill at import of the next snapshot, **or** a one-shot backfill on startup / a small CLI. Migration SQL cannot call the Rust crate. |

Import must re-derive when the field is absent. Prod applies snapshots/diffs
and never scrapes; old export JSON will not have `country`.

## API

`GET /superchargers/soon`

| Param | Behavior |
|---|---|
| `?country=DE` | `WHERE country = 'DE'` (normalize to uppercase) |
| `?country=germany` | `400` unless we add a name alias table (Q6) |
| `?country=UK` | `400` (or alias to `GB` — Q5) |
| `?country=XX` unknown ISO | `400` if we validate against a known set; otherwise empty list |
| `?region=` | Unchanged |
| both | `AND` |

Unknown / empty `country` rows are excluded from a country filter (they do
not match `DE`). That is correct: we cannot claim they are in that country.

The public read API does **not** list `opened_superchargers` today. Country
search on opened sites is export/DB-only unless we add a new route (Q2).

## Implementation surface (small, but wide)

- `Cargo.toml` — add `country-boundaries`
- New helper, e.g. `src/domain/geo.rs` — singleton + `fn country_from_coords(lat, lng) -> Option<String>`
- Domain: `ComingSoonSupercharger.country`
- Migration: add `country TEXT` + index on both charger tables
- Repository upserts, unchanged updates, graduation INSERT, list SQL
- `export.rs` types (`ExportChangedCharger`, `ExportOpenedCharger`)
- `application/export_{diff,snapshot}.rs` SELECTs
- Import upserts
- `api/superchargers.rs` — field + `?country=`
- Docs: `docs/API.md`, `docs/ARCHITECTURE.md`, `AGENTS.md` schema notes
- Verify skill: fixture rows get `country` (`GB` for `11255`, `US` for `12001`); new `list-country` case
- Unit tests for the picker (Dallas → `US`, Highbridge → `GB`, ocean → `None`)

No Chrome / Tesla scrape needed to ship or prove this.

## Non-goals

- Replacing or “fixing” `region`
- Storing subdivision (`US-TX`) — that is still `region` for the US
- Geocoding city / street
- Changing `scrape --country` or `scrape_runs.country`
- A public opened-chargers list (unless Q2 says yes)

## Open questions

### Q1 — Column name: `country` vs something else?

`country` matches how we want to query. Alternatives: `derived_country`,
`country_iso`. We already have Tesla `country_code` and `scrape_runs.country`,
so three “country” concepts will exist. Is `country` still the right name on
the charger row?

**Proposal:** `country` on the charger tables and JSON. Document the three
meanings.

### Q2 — Do opened chargers need to be searchable via HTTP?

The stated goal is “search all chargers by country.” The public API only
lists coming-soon. Opened rows live in `opened_superchargers` and leave
`GET /superchargers/soon`.

**Proposal:** Persist `country` on opened rows now (graduation + export).
Do **not** add an opened list endpoint in this change unless you want it.

### Q3 — `NULL` vs nearest-land when the point is in water?

`country-boundaries` only claims correctness on land. Coastal Tesla pins
sometimes sit a few meters offshore.

**Proposal:** Store `NULL` and log a warning. Do not snap to nearest country
in v1. If real scrapes show a meaningful miss rate, add a small fallback
later (e.g. try a few meters inland, or use Tesla `country_code`).

### Q4 — Overlapping / disputed claims?

`ids()` can return more than one ISO-2 code (border, exclave, disputed).
Examples that may matter for this dataset: Taiwan, Kosovo, Israel /
Palestine, Crimea, Western Sahara.

**Proposal:** Take the **largest-area** ISO-2 in `ids()` (last 2-letter
code, since the crate orders smallest-first). Do not try to encode
politics. Call this out in docs. If you have a preferred code for a
specific territory, say so before implementation.

### Q5 — `UK` vs `GB` in the API?

ISO 3166-1 is `GB`. People will type `UK`. Tesla titles say `United Kingdom`
/ `UK`.

**Proposal:** Store `GB`. Accept `UK` as an alias on `?country=` only
(`UK` → `GB`). Document it next to the current `?region=UK` behavior.

### Q6 — ISO-2 only, or also country names on `?country=`?

`?region=` already accepts `Germany`, `United Kingdom`, etc.

**Proposal:** `?country=` is ISO-2 plus the `UK` → `GB` alias. No name
table. Names stay on `?region=`. Country search is the reliable ISO path.

### Q7 — Validate unknown codes?

**Proposal:** Reject anything that is not `[A-Za-z]{2}` (plus `UK`) with
`400`. Do not maintain a full ISO country allow-list. `?country=ZZ` that
matches no rows returns `total: 0`, not `400`.

### Q8 — Backfill existing prod rows before the next scrape/import?

A SQL migration cannot run the crate. Options:

- A) Next snapshot/diff import re-derives (prod waits until the next local export)
- B) Host derives on startup for any `country IS NULL` row
- C) One-shot CLI (`backfill-country`) run against prod’s DB

**Proposal:** A + derive-on-import for missing fields. Add B only if prod
must be queryable by country before the next import.

### Q9 — Should `?country=` also filter recent-* and map?

**Proposal:** List + map in this change (map is the other “all chargers”
read). Recent-* stay unfiltered. Detail just returns the field.

### Q10 — ODbL attribution — where?

The bundled polygons are OpenStreetMap / ODbL. Attribution is required.

**Proposal:** One line in README and `docs/ARCHITECTURE.md` data-loading
notes. Not on every API response.

### Q11 — Next.js consumer

The verify skill says the Next.js app calls this API. This repo does not
contain that app.

**Proposal:** Ship the Rust column + `?country=` + response field. Frontend
can switch from `?region=Germany` to `?country=DE` in a follow-up. Confirm
whether the frontend must land in the same change.

## Suggested implementation order

1. Agree Q1–Q6 (name, opened HTTP, NULL/overlap, `UK`/`GB`, filter syntax).
2. Crate + `country_from_coords` + unit tests.
3. Migration + domain/repo/export/import write path.
4. `?country=` on list (and map if Q9 stays yes) + response field.
5. Docs, fixtures, verify-skill list-country case.
6. Operator proof via `.cursor/skills/verify-tesla-superchargers/` (no live scrape).

## Risks

- Coastal pins → `NULL` (Q3). Low for inland Superchargers; possible for
  harbor / island sites.
- `GB` vs `UK` will surprise anyone copying today’s `?region=UK` habit (Q5).
- Three country-like fields (`region`, `country_code`, `country`) until
  docs are clear.
- Default OSM dataset encodes OSM’s view of disputed borders (Q4).
- Binary size: `BOUNDARIES_ODBL_360X180` is the largest/fastest raster.
  Acceptable for this binary; switch to `180x90` only if size becomes a
  problem.
