# Plan: Capture richer planned-charger details

**Status:** Decisions complete — ready to implement
**Date:** 2026-06-23
**Related:** `docs/scrape-findings.html`

## Goal

Capture additional fields Tesla already returns for coming-soon (planned) chargers, at no
extra HTTP cost, persist them, and expose them through the API.

| Field | Source (in detail response) | Example | Type in DB |
|---|---|---|---|
| `project_status` | `supercharger_function.project_status` | `Design`, `Construction` | drives the derived `status` (TEXT) **and** stored raw as `raw_project_status` (TEXT) — see D0 |
| `num_charger_stalls` | `supercharger_function.num_charger_stalls` | `"8"` → `8` | `INTEGER NOT NULL DEFAULT 0` |
| `charging_accessibility` | `supercharger_function.charging_accessibility` | `Tesla Only` | `TEXT` |
| `real_estate_status` ⏸️ | `functions[0].status` | `Lease Negotiations` | `TEXT` — **deferred from v1, see D6** |
| `street_address` | `functions[0].address.address_1` | `5 Via Sergio Fraccalanza` | `TEXT` |
| `county` | `functions[0].address.county` | `Provincia di Padova` | `TEXT` |
| `postal_code` | `functions[0].address.postal_code` | `35129` | `TEXT` |
| `country_code` | `functions[0].address.country` | `IT` | `TEXT` |

All come from the **same** detail response unlocked by the query change below — no extra calls.

> **⏸️ `real_estate_status` is deferred from v1 (D6).** It's `functions[0].status` — the lease/property
> deal stage — decoupled from build progress and with no consumer yet, so it's left out for now. The
> *address* fields (also from `functions[0]`) stay. It can be added later at zero fetch cost since it
> rides in the same response. Implementation details for it are kept below, gated on D6.

### Source-of-truth precedence (important)

**`supercharger_function` is authoritative. `functions[]` is complementary only.** When the two
overlap or conflict, always trust `supercharger_function`.

This matters because `functions[]` describes the *real-estate / leasing* function, whose vocabulary
overlaps with charger-status words but means something different. Concrete example — slug `30277`
(Denkendorf BY, Germany):

| | value |
|---|---|
| `supercharger_function.site_status` | `coming_soon` |
| `supercharger_function.project_status` | `Design` |
| `supercharger_function.customer_facing_coming_soon_date` | `In Development` |
| `functions[0].status` | **`Open`** ← leasing/RE status, NOT charger open |

The charger is plainly still in development; `functions[0].status: "Open"` refers to the leasing
function, not the Supercharger. So:

- **Charger lifecycle / status / stalls / accessibility → always from `supercharger_function`.**
- **`functions[]` is used only for data that doesn't exist in `supercharger_function`:** the structured
  address, and `real_estate_status` as **informational metadata only**. Never derive `site_status`,
  `project_status`, "is it open," or any lifecycle decision from `functions[].status`.

**The lease pipeline is decoupled from the build pipeline.** We've observed the two move out of step in
both directions, so `functions[].status` is a parallel real-estate workflow, not a measure of build
progress:

| Slug | `project_status` (build) | `functions[0].status` (lease) |
|---|---|---|
| `30277` Denkendorf | `Preliminary` / `coming_soon` | `Open` |
| `19009` Camp Verde, AZ | `Construction` | `Lease Negotiations` |

This reinforces the precedence rule (never let `real_estate_status` inform lifecycle) **and** casts
doubt on ranking lease values into a monotonic pipeline — see D6/D7.

### The three status-like fields (`project_status` vs `site_status` vs customer-facing)

Tesla exposes three overlapping "status" concepts at different granularities. We currently derive our
`SiteStatus` from the **customer-facing** one; `project_status` is finer and is the main value-add.

| Field | Granularity | Observed values | Meaning |
|---|---|---|---|
| `site_status` | coarse (system bucket) | `coming_soon`, `open` | Which Find Us feed the location lives in. Flips to `open` at launch. |
| `project_status` | fine (build pipeline) | `Preliminary` → `Design` → `Construction` → `Open` | How far the *build* has progressed while still `coming_soon`. |
| `customer_facing_coming_soon_date` | public label | `In Development`, `Under Construction` | The string shown to users; **what our current `SiteStatus` is derived from**. |

`project_status` is a sub-state of `site_status = coming_soon`, and is strictly finer than the
customer-facing label: both `Preliminary` and `Design` surface publicly as `In Development`, while
`Construction` surfaces as `Under Construction`. So today we **cannot** tell `Preliminary` (earliest
— site picked/voted, often null coords, `0` stalls; e.g. community-vote winners) apart from `Design`
(planning underway) — both collapse to `In Development`. Capturing `project_status` fixes that.

Example — slug `451205` (Garmisch-Partenkirchen, a vote winner, `vote_winner_quarter: Q1 2023`):
`site_status = coming_soon`, `project_status = Preliminary`,
`customer_facing_coming_soon_date = In Development`, `actual_latitude/longitude = null`,
`num_charger_stalls = "0"`.

### Status-model integration — ✅ **DECIDED (D0): re-model around `project_status`, stored as TEXT**

`project_status` becomes **the** status, in a single vocabulary used everywhere (DB, domain, API — no
remapping at any layer), stored as **`TEXT`** and validated in the app (no Postgres enum), derived
entirely on scan.

**Status vocabulary (canonical casing = ALL CAPS):** `PRELIMINARY`, `DESIGN`, `CONSTRUCTION`, `OPENED`,
`UNKNOWN`, `REMOVED` — this is exactly what's stored in `status` and returned by the API
(`SiteStatus` keeps `rename_all = "SCREAMING_SNAKE_CASE"`, consistent with the old `IN_DEVELOPMENT`
contract). `OPENED` stays owned by the existing graduation/open-detection flow; coming-soon rows are
only ever `PRELIMINARY`/`DESIGN`/`CONSTRUCTION`/`UNKNOWN`. *(Title-case `Preliminary` etc. elsewhere in
this doc is informal prose — and is also the raw Tesla casing stored separately in `raw_project_status`.
So two casings coexist by design: ALL-CAPS derived `status`, title-case raw values + `KNOWN_PROJECT_STATUS`.)*

**Derivation on scan** (`customer_facing` is the fallback when `project_status` is absent/unknown):

```
project_status →
  "Preliminary"   => Preliminary
  "Design"        => Design
  "Construction"  => Construction
  "Open"          => fall back to customer_facing   // don't pre-open; open-detection owns Opened
  missing/unknown => fall back to customer_facing    // + warn-on-unknown

customer_facing fallback →
  "In Development"     => Preliminary
  "Under Construction" => Construction
  else                 => Unknown
```

**Columns:** keep `raw_status_value` (= customer_facing) and add `raw_project_status` (raw Tesla value);
`status` holds the derived value. `site_status` is **not stored** (D10 — redundant; coming_soon/open is
already in the lifecycle; could later sharpen open-detection as a separate enhancement).
`real_estate_status` is **deferred** (D6).

**Why `TEXT`, not a Postgres enum:** consistent with the other new TEXT fields + warn-on-unknown; future
status-vocabulary changes need no DB migration; all writes are app-controlled, so app-level enforcement
(the `SiteStatus` enum via `FromStr`/`Display`) suffices. A one-time migration is still needed now to
convert the existing `site_status` enum → TEXT and remap values.

> ⚠️ **Naming note:** Tesla's API field `site_status` is **not** our domain `SiteStatus`. We're not
> storing `site_status`, so no collision — but keep the distinction in mind when reading Tesla data.

**Day-one re-classification (sub-decision — ✅ decided (a)):** the migration backfill maps
`IN_DEVELOPMENT → PRELIMINARY` and `UNDER_CONSTRUCTION → CONSTRUCTION` (best guess). The first scrape
after deploy re-derives real `project_status`, so Design-stage chargers flip `PRELIMINARY → DESIGN`
and emit `status_changes` — a one-time burst of discovery events. **Accept the burst;** no suppression
logic.

**Side effect — largely resolves D5:** because `project_status` transitions are now real `status_changes`,
they flow to prod via the existing diff automatically. The only remaining propagation gap is fields that
change *without* a status change (`num_charger_stalls`, `charging_accessibility`, address).

The implementation steps below reflect this decision.

### `num_charger_stalls`: 0 means "unknown"

Per decision: store `num_charger_stalls` as `INTEGER NOT NULL DEFAULT 0`, and map a missing/unparseable
value to `0`. Tesla itself already returns `"0"` for ~20% of planned sites (a site whose stall count
isn't decided yet), so we deliberately **conflate** "Tesla said 0" and "we have no value" into a
single sentinel: **`0` = unknown / not yet decided.** Consumers should treat `0` as "no published
count," not "a Supercharger with zero stalls."

### Field reliability (measured)

Two live samples via `functionTypes=coming_soon_supercharger,supercharger`, **0 fetch errors** in both:

| Field | Present | Notes |
|---|---|---|
| `project_status` | **100%** | Always set (see value catalog below). |
| `num_charger_stalls` | 100% present, **~80% non-zero** | The ~20% zeros span all stages — genuine "not decided," not a fetch gap. Mapped to the `0 = unknown` sentinel. |
| `charging_accessibility` | **~93%** | Small fraction unset. |
| `functions[0].status` | **100%** | Real-estate / leasing pipeline — **deferred (D6), not in v1.** |
| `functions[0].address` | **100%** | Full structured address. |
| `installed_full_power` | 100% present, **0% non-zero** | **Always `"0"` for planned sites** — dropped, see below. |

### Not on coming-soon table

- **`installed_full_power` — not on `coming_soon_superchargers`.** Always `"0"` for planned sites.
  **D11 — ✅ decided:** capture at graduation into `opened_superchargers` as `installed_full_power_kw`
  (INTEGER, kW parsed from open-check `supercharger_function.installed_full_power`). See
  [step 6](#6-graduation--installed_full_power-d11) below.
- **`actual_latitude` / `actual_longitude` — out of scope.** `null` or coarsely rounded for planned
  sites; the existing feed coordinate already equals the more precise
  `coming_soon_latitude/longitude`. We keep the current feed coordinates. The best precise-coordinate
  candidate, if ever wanted, is `functions[0].address.latitude/longitude` (already being read for the
  address — could be added cheaply later).

---

## Known enum-like values (observed)

Catalogued from a live scan of **239 planned superchargers** worldwide (0 fetch errors). Counts are
indicative, not exhaustive — see the warning-strategy section: rare values (e.g. `Tesla Signed`,
1/239) show that more values almost certainly exist beyond what a sample reveals.

**`project_status`** — construction pipeline:

| Value | Count |
|---|---|
| `Design` | 88 |
| `Construction` | 87 |
| `Preliminary` | 60 |
| `Open` | 4 |

(`Open` appears because a site can flip to open while still briefly listed in the coming-soon feed.)

**`functions[0].status`** — real-estate / leasing pipeline. **Deferred (D6), not stored in v1** —
catalogued here for reference only. Informational, **not** charger lifecycle (decoupled from build
progress); `Open` (17) does **not** mean the charger is open.

| Value | Count |
|---|---|
| `Pending` | 162 |
| `Lease Negotiations` | 55 |
| `Open` | 17 |
| `Fully Executed` | 4 |
| `Tesla Signed` | 1 |

**`charging_accessibility`:**

| Value | Count |
|---|---|
| `Tesla Only` | 154 |
| `All Vehicles (Production)` | 38 |
| `NACS Partner Enabled (Production)` | 30 |
| (unset) | 17 |

**`access_type`** (not currently stored; catalogued for reference): `Public` (225), `Service` (1),
`None` (2), unset (11).

---

## Handling unknown / new values (warn-on-unknown) — proposed

These fields look enum-like but we **cannot prove the value set is closed** — Tesla can introduce a
new `project_status` or leasing stage at any time, and our sample only saw 239/703 sites.

**Proposal:** store all enum-like fields as plain `TEXT` (not a Postgres `ENUM`), so an unexpected
value never breaks ingestion or requires a migration. In the domain mapping, validate each value
against a known-set constant and emit `tracing::warn!` when a value isn't recognised, so we notice
and can add it deliberately.

This mirrors the existing pattern in `SiteStatus::from_opt` (`src/domain/coming_soon.rs:59`), which
already logs `"unrecognised site status — defaulting to Unknown"`. Example:

```rust
const KNOWN_PROJECT_STATUS: &[&str] =
    &["Preliminary", "Design", "Construction", "Open"];

fn validate_project_status(value: &str) {
    if !KNOWN_PROJECT_STATUS.contains(&value) {
        tracing::warn!(value, "unrecognised project_status — consider adding to KNOWN_PROJECT_STATUS");
    }
}
```

Apply the same to `charging_accessibility`. The raw string is still stored verbatim regardless — the
warning is purely an observability signal, never a data filter.

### When and how to surface the warning

A naive `warn!` per charger would print the same unknown value hundreds of times and bury it. Use a
**deduped immediate warn + an end-of-run summary**, so a new value is both visible the moment it
appears *and* impossible to miss after the run scrolls past.

Model it on the existing `failure_reasons: HashMap<String, usize>` aggregation already on
`DetailBatchFetchResult` (`src/scraper/loaders.rs:49`):

```rust
#[derive(Default)]
pub struct UnknownEnumTracker {
    /// (field, raw value) -> count seen this run
    seen: HashMap<(&'static str, String), usize>,
}

impl UnknownEnumTracker {
    /// Call for every value of an enum-like field. Warns immediately the FIRST time
    /// each distinct (field, value) is seen; counts every occurrence for the summary.
    fn record(&mut self, field: &'static str, value: &str, known: &[&str]) {
        if known.contains(&value) { return; }
        let entry = self.seen.entry((field, value.to_string())).or_insert(0);
        if *entry == 0 {
            tracing::warn!(field, value, "unrecognised enum value — first seen this run");
        }
        *entry += 1;
    }

    /// Logged once at the end of the run; no-op if everything was recognised.
    fn log_summary(&self) {
        if self.seen.is_empty() { return; }
        for ((field, value), count) in &self.seen {
            tracing::warn!(field, value, count, "unrecognised enum value (run total)");
        }
        tracing::warn!(distinct = self.seen.len(), "run saw unrecognised enum values — review and extend the KNOWN_* sets");
    }
}
```

**Wiring (the "save them and print at the end" part):**

1. Own one `UnknownEnumTracker` in `fetch_batch_details_from_page` (`src/scraper/loaders.rs`), which
   loops over all detail batches. Pass `&mut` into the per-batch parse (`classify_detail_pairs` /
   `detail_from_value`) so dedup and counts persist across batches — the immediate warns fire during
   the scrape as each new value first appears.
2. Move the finished tracker onto `LoadResult` (alongside `failed_detail_ids`).
3. In `run_scrape` (`src/application/scrape.rs`), call `tracker.log_summary()` right after the
   existing "DB update complete" summary log (~line 120), so the run ends with a consolidated roll-up
   even if the inline warns scrolled away.

This gives exactly the requested behaviour: **immediate (deduped) during the run, plus a saved
summary at the end.** A clean run logs nothing extra.

**D3 — not persisted:** no `JSONB` column on `scrape_runs` for v1 (logs only per D2/D3).

**D1 — ✅ decided (a):** plain `String` + const `KNOWN_*` sets + deduped `warn!` for enum-like fields
(`raw_project_status`, `charging_accessibility`, …). Derived `status` keeps the `SiteStatus` enum.

**D2 — ✅ decided:** logs only for v1 (deduped `warn!` + end-of-run summary); no CLI/API surfacing.

**D3 — ✅ decided:** don't persist unknown-value summary on `scrape_runs` for v1.

---

## Regression handling: keep the better value

Tesla's feed sometimes reports a *less* advanced or empty value for a field that was previously more
specific — a transient blip, a partially-populated record, or a re-sync. We don't want a later scrape
to **erase progress**. Two rules, applied per field at write time.

### Rule A — never overwrite a known value with "unknown"

A *successful* fetch can still return an empty/sentinel value for an individual field. Don't let that
clobber a value we already have. (This is distinct from the existing preserve-on-failure guard, which
only fires when the whole detail fetch fails — here the fetch succeeded but one field came back empty.)

| Field | "unknown" sentinel | Behaviour |
|---|---|---|
| `num_charger_stalls` | `0` | keep existing if existing `> 0` |
| `charging_accessibility` | `NULL` / unset | keep existing |
| `raw_project_status` | `NULL` | keep existing |
| `street_address` / `county` / `postal_code` / `country_code` | `NULL` | keep existing |

(`status` is derived — Rule A applies to the raw columns above; regression policy for the derived
`status` itself is separate, see [D8](#d8--status-regression-policy-decided) below.)

SQL shape (in both `ON CONFLICT DO UPDATE` and the unchanged `UPDATE`):
`col = CASE WHEN EXCLUDED.col IS NULL THEN <existing>.col ELSE EXCLUDED.col END`
(`= 0` instead of `IS NULL` for `num_charger_stalls`).

Rule A is low-risk and recommended for all the fields above unconditionally.

### Rule B — don't regress within an ordered pipeline (ratchet)

**Not used in v1** for stored columns. Rule B was conceived for `real_estate_status` (deferred, D6).
Derived `status` uses a narrower hybrid policy instead (D8) — not a full ratchet.

### D8 — status regression policy — ✅ **DECIDED**

`derive_status` produces a **candidate** from the scrape; `resolve_status_transition(existing, candidate, …)`
applies these rules before `compute_sync` compares old vs new (domain logic, not SQL `CASE`):

| Situation | Action |
|---|---|
| Forward move (`PRELIMINARY→DESIGN`, `DESIGN→CONSTRUCTION`, …) | **Always take it** |
| Derivation yields `UNKNOWN` | **Keep existing** — unknown ≠ backward |
| `project_status` missing; `customer_facing` still `"In Development"`; existing status is finer (`DESIGN`) | **Keep existing** — coarse label covers both `PRELIMINARY` and `DESIGN`; fallback to `PRELIMINARY` is not a regression signal |
| Fallback upgrades (`DESIGN` + only `"Under Construction"`) | **Take `CONSTRUCTION`** — coarse fallback may move forward, not backward |
| Explicit `project_status` backward (`CONSTRUCTION→DESIGN`, …) | **Record + `warn!`** — may be real; don't silently ratchet away |

### "Any other things like that?" — per-field decisions

- **`num_charger_stalls`** — Rule A only (keep a known count over the `0` = unknown sentinel). No
  pipeline order.
- **derived `status`** — D8 hybrid policy above (not a blanket ratchet).
- **`site_status` / our `SiteStatus`** — ✅ **D9 moot** (D0 + D8): audited `status` is the derived
  `project_status` field; Tesla `site_status` not stored. Never ratchet.
- **`charging_accessibility`** — Rule A only. Its real values (`Tesla Only` / `All Vehicles` / `NACS
  Partner Enabled`) are genuine policy changes, not a pipeline, so a change between them should be
  recorded, not suppressed.
- **address fields** — Rule A only (a corrected address should be taken; only an empty one is ignored).

### Implementation note

Rule A compares against the existing row, so these writes can no longer be blind overwrites: use
`CASE` expressions in the upsert's `ON CONFLICT DO UPDATE` and in the unchanged `UPDATE`, referencing
`coming_soon_superchargers.<col>` / `cs.<col>`. **Mirror the same `CASE` logic in
`save_chargers_from_diff` and `apply_snapshot`.** This composes with the preserve-on-failure guard
(failed fetch → keep everything; successful fetch → apply Rule A per field). Derived `status` uses D8 in
domain code before sync.

---

## Implementation steps

### 1. Fetch layer

**`src/scraper/loaders.rs`** — in `eval_detail_batch` (~line 644), change the query string so the
response includes `supercharger_function` **and** `functions[]`:

```diff
- fetch(`/api/findus/get-location-details?locationSlug=${slug}&functionTypes=coming_soon_supercharger&locale=en_US&isInHkMoTw=false`,
+ fetch(`/api/findus/get-location-details?locationSlug=${slug}&functionTypes=coming_soon_supercharger,supercharger&locale=en_US&isInHkMoTw=false`,
```

(Requesting `supercharger` alone on a planned site returns nothing — both types are required.)

**`src/scraper/raw.rs`** — the response carries the new data in two sibling objects:
`supercharger_function` (stall / status / accessibility) and `functions[0]` (structured address). Add
raw types for `functions[]` and merge them into `ComingSoonDetails`. (`functions[0].status` exists but
is deferred per D6, so it's intentionally not deserialized.)

```rust
#[derive(Deserialize)]
pub struct LocationDetailsData {
    pub supercharger_function: Option<RawSuperchargerFunction>,
    #[serde(default)]
    pub functions: Option<Vec<RawFunction>>,
}

#[derive(Deserialize)]
pub struct RawSuperchargerFunction {
    pub customer_facing_coming_soon_date: Option<String>,
    pub coming_soon_name: Option<String>,
    pub project_status: Option<String>,
    pub num_charger_stalls: Option<String>,       // string in the API
    pub charging_accessibility: Option<String>,
}

#[derive(Deserialize)]
pub struct RawFunction {
    pub address: Option<RawAddress>,
    // `status` (lease/real-estate stage) exists here but is deferred — D6.
}

#[derive(Deserialize)]
pub struct RawAddress {
    pub address_1: Option<String>,
    pub county: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}
```

`ComingSoonDetails` becomes the merged, parsed carrier the rest of the code consumes:

```rust
#[derive(Clone, Default)]
pub struct ComingSoonDetails {
    pub customer_facing_coming_soon_date: Option<String>,
    pub coming_soon_name: Option<String>,
    pub project_status: Option<String>,
    pub num_charger_stalls: Option<String>,
    pub charging_accessibility: Option<String>,
    pub street_address: Option<String>,
    pub county: Option<String>,
    pub postal_code: Option<String>,
    pub country_code: Option<String>,
}
```

In `detail_from_value` (`src/scraper/loaders.rs`), build `ComingSoonDetails` by combining
`supercharger_function` with `functions.first()` (address only). Run the `validate_*` warn checks here.

**Honour the precedence rule when merging:** take `project_status`, `num_charger_stalls`,
`charging_accessibility`, and anything lifecycle-related strictly from `supercharger_function`. Pull
only `street_address` / `county` / `postal_code` / `country_code` from `functions[0]`. If
`supercharger_function` is absent, treat the location as having no usable details (same as today) — do
**not** fall back to anything in `functions[]` to infer a charger status.

### 2. Domain model — status is now **derived** (D0)

**`src/domain/coming_soon.rs`** — rework the `SiteStatus` enum to the new vocabulary and store it as
TEXT (drop the `#[sqlx(type_name = "site_status", …)]` enum mapping; map via `Display` / `FromStr`):

```rust
pub enum SiteStatus {
    Preliminary,    // was InDevelopment (earliest)
    Design,         // new
    Construction,   // was UnderConstruction
    Unknown,
    Removed,
    Opened,         // owned by the graduation/open-detection flow
}
```

Add a `derive_status` that prefers `project_status`, falling back to `customer_facing` (with
warn-on-unknown), per D0. Then `resolve_status_transition(existing, candidate, raw_project_status, …)`
applies D8 before persisting:

```rust
fn derive_status(project_status: Option<&str>, customer_facing: Option<&str>) -> SiteStatus { /* D0 */ }
fn resolve_status_transition(
    existing: SiteStatus,
    candidate: SiteStatus,
    raw_project_status: Option<&str>,
) -> SiteStatus {
    // Forward: always take candidate when rank(candidate) > rank(existing)
    // UNKNOWN candidate: keep existing
    // Fallback-only regression (no raw project_status; customer_facing "In Development";
    //   candidate PRELIMINARY but existing is DESIGN): keep existing
    // Explicit backward raw project_status: take candidate + warn!
}
```

Stored/API values use **uppercase** (`PRELIMINARY`, `DESIGN`, `CONSTRUCTION`, …) via `Display`/`FromStr`.

Add the descriptive fields to `ComingSoonSupercharger`, including `raw_project_status` (the raw Tesla
value; `raw_status_value` continues to hold raw `customer_facing`):

```rust
pub status: SiteStatus,                  // derived (existing field, new vocabulary)
pub raw_status_value: Option<String>,    // = customer_facing (existing)
pub raw_project_status: Option<String>,  // new
pub num_charger_stalls: i32,             // 0 = unknown (see semantics above)
pub charging_accessibility: Option<String>,
pub street_address: Option<String>,
pub county: Option<String>,
pub postal_code: Option<String>,
pub country_code: Option<String>,
```

Populate in both `from_location` and `with_details`:

```rust
status: resolve_status_transition(
    existing.status,
    derive_status(details.and_then(|d| d.project_status.as_deref()),
                  details.and_then(|d| d.customer_facing_coming_soon_date.as_deref())),
    details.and_then(|d| d.project_status.as_deref()),
), // in from_location, existing is Unknown / absent
raw_status_value: details.and_then(|d| d.customer_facing_coming_soon_date.clone()),
raw_project_status: details.and_then(|d| d.project_status.clone()),
num_charger_stalls: details
    .and_then(|d| d.num_charger_stalls.as_deref())
    .and_then(|s| s.parse().ok())
    .unwrap_or(0),                        // missing/unparseable → 0 = unknown
charging_accessibility: details.and_then(|d| d.charging_accessibility.clone()),
street_address: details.and_then(|d| d.street_address.clone()),
county: details.and_then(|d| d.county.clone()),
postal_code: details.and_then(|d| d.postal_code.clone()),
country_code: details.and_then(|d| d.country_code.clone()),
```

Also update: the `charger(...)` test helper in `src/domain/sync.rs`; the two row→struct closures in
`get_failed_detail_chargers` / `get_failed_open_status_chargers`; and every query that casts
`::site_status` or compares against `'IN_DEVELOPMENT'` / `'UNDER_CONSTRUCTION'` — those become plain
text comparisons against `'PRELIMINARY'` / `'CONSTRUCTION'` etc.

> **D4 — ✅ decided:** keep title parsing for `city`/`region` in v1. Store structured address columns
> (`street_address`, `county`, `postal_code`, `country_code`) only; don't derive `city`/`region` from
> `functions[0].address` yet (separate follow-up if wanted).

### 3. Migration

New file `migrations/<timestamp>_planned_charger_details.sql`. Two parts: (a) convert the
`site_status` enum column to TEXT and remap values (the `USING CASE` is the backfill — it rewrites both
live data and history in one pass, no separate UPDATE); (b) add the new descriptive columns.

```sql
-- (a) site_status enum → TEXT, remapping the renamed values
ALTER TABLE coming_soon_superchargers ALTER COLUMN status DROP DEFAULT;
ALTER TABLE coming_soon_superchargers
    ALTER COLUMN status TYPE TEXT USING (
        CASE status::text
            WHEN 'IN_DEVELOPMENT'     THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION'
            ELSE status::text             -- UNKNOWN / REMOVED / OPENED unchanged
        END);
ALTER TABLE coming_soon_superchargers ALTER COLUMN status SET DEFAULT 'UNKNOWN';

ALTER TABLE status_changes
    ALTER COLUMN old_status TYPE TEXT USING (
        CASE old_status::text
            WHEN 'IN_DEVELOPMENT' THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION' ELSE old_status::text END),
    ALTER COLUMN new_status TYPE TEXT USING (
        CASE new_status::text
            WHEN 'IN_DEVELOPMENT' THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION' ELSE new_status::text END);

DROP TYPE site_status;   -- column no longer references it

-- (b) new descriptive columns
ALTER TABLE coming_soon_superchargers
    ADD COLUMN raw_project_status    TEXT,
    ADD COLUMN num_charger_stalls    INTEGER NOT NULL DEFAULT 0,  -- 0 = unknown
    ADD COLUMN charging_accessibility TEXT,
    ADD COLUMN street_address        TEXT,
    ADD COLUMN county                TEXT,
    ADD COLUMN postal_code           TEXT,
    ADD COLUMN country_code          TEXT;
```

Notes: `old_status` is nullable — `NULL::text` passes through the `CASE` as `NULL` (correct). The index
on `status` is rebuilt automatically by the type change. `num_charger_stalls` is `NOT NULL DEFAULT 0`
per the sentinel; the rest nullable. All transactional; runs on startup via `sqlx::migrate!()`.

The `(a)` remap is a best guess (`IN_DEVELOPMENT → PRELIMINARY`); the first scan then corrects
Design-stage rows (`PRELIMINARY → DESIGN` burst accepted per D0 sub-decision).

### 4. Persistence — **both** write paths (the important part)

In `compute_sync` (`src/domain/sync.rs`), an existing charger lands in `upserts` only when its
**status** changes; otherwise it lands in `unchanged` — the majority every scrape. So the new
columns must be written in **both** paths in `save_chargers` (`src/repository/supercharger.rs`):

- **Upsert INSERT** (~lines 169–218): add the new columns to the column list, the `unnest(...)`
  SELECT, the `ON CONFLICT DO UPDATE SET`, and bind the new arrays.
- **Unchanged UPDATE** (~lines 233–257): add the new columns to the `SET` and the `FROM (SELECT
  unnest...)` virtual table, and bind the new arrays.

**Preserve-on-failure guard:** the unchanged path already keeps title/city/region when the detail
fetch failed this run (`CASE WHEN cs.id = ANY($failed) THEN cs.<col> ELSE v.<col> END`). Apply the
same guard to the new columns so a failed fetch never overwrites good values (and never resets
`num_charger_stalls` back to the `0` sentinel). The upsert path uses the equivalent
`CASE WHEN EXCLUDED.details_fetch_failed THEN ...`.

**Regression guard:** on top of the preserve-on-failure guard, these columns also need the per-field
`CASE` logic from the [Regression handling](#regression-handling-keep-the-better-value) section
(Rule A for all new columns; no Rule B / ratchet in v1). Same `CASE` must be mirrored in
`save_chargers_from_diff` and `apply_snapshot`.

### 5. API exposure

- **`src/repository/models.rs`** — add the new fields to `ApiSupercharger` (and `ApiRecentAddition`
  if desired).
- **`src/repository/supercharger.rs`** — add the columns to the `SELECT` lists in `list_coming_soon`
  (both branches), `get_coming_soon`, and any read mapping to `ApiSupercharger`; map them in the
  row→model closures. The `status` value is passed through directly — no remapping (D0).
- **Status filter + stats (D0 ripple):** the `list_coming_soon` status filter param and `DbStats`
  buckets now use `PRELIMINARY` / `DESIGN` / `CONSTRUCTION` instead of `IN_DEVELOPMENT` /
  `UNDER_CONSTRUCTION` (`get_db_stats` / `count_coming_soon_by_status`). **Frontend** must update the
  values it filters by, labels, and any status counts it displays.
- **`docs/API.md`** — document the new response fields (note `num_charger_stalls: 0` = unknown) and the
  new `status` values.

### 6. Graduation — `installed_full_power` (D11)

Not stored on coming-soon rows. Captured once when a charger graduates via the existing open-check
path (`functionTypes=supercharger` in `check_open_status` / `loaders.rs`).

**Migration** (same file): add to `opened_superchargers`:

```sql
ALTER TABLE opened_superchargers
    ADD COLUMN installed_full_power_kw INTEGER;  -- NULL if missing/unparseable/"0"
```

**Fetch / domain:**

- `OpenCheckSuperchargerFunction` (`src/scraper/raw.rs`): add `installed_full_power: Option<String>`.
- Open-check parse (`loaders.rs`): parse to `i32`; treat missing, unparseable, and `"0"` as `None`.
- `OpenResult` (`src/domain/sync.rs`): add `installed_full_power_kw: Option<i32>`.

**Persistence:**

- Graduation `INSERT INTO opened_superchargers` in `save_chargers` (`supercharger.rs`) — bind the new field.
- Mirror in `save_chargers_from_diff` and `apply_snapshot` opened-charger inserts.

**Import/export:**

- `ExportOpenedCharger` (`export.rs`): add
  `#[serde(default, skip_serializing_if = "Option::is_none")] pub installed_full_power_kw: Option<i32>`.
- `get_opened_chargers_for_run`, `export-snapshot` SELECT, `row_to_export_opened` — include the column.

**Frontend (separate repo):** if opened chargers are shown anywhere, surface `installed_full_power_kw`
(e.g. "250 kW") when present.

---

## Answer: do defaulted columns get backfilled automatically?

**Yes — no separate backfill job is needed, provided step 4 is done.** The migration adds the
columns (`NULL`, or `0` for stalls). On the next scrape:

- Every charger present in the feed is processed by `compute_sync` and routed to `upserts` or
  `unchanged`. Both paths (once updated) write the new fields.
- The `unchanged` UPDATE already runs for every still-present charger each scrape (it refreshes
  title/city/region/`last_scraped_at` today), so existing rows get populated on the **first run
  after deploy**, not just when a status changes.

Edges, none needing special handling: a charger whose detail fetch keeps failing stays at defaults
until a successful fetch (correct — we never invent data); REMOVED tombstones are never updated (they
are absent from the feed).

---

## Answer: import/export changes

Yes — and there is a propagation gap to decide on.

### Required mechanical changes (so prod can receive the fields)

1. `src/export.rs` — add the new fields to `ExportChangedCharger`. Use
   `#[serde(default, skip_serializing_if = "Option::is_none")]` (and `#[serde(default)]` for the
   non-optional `num_charger_stalls`) so older export files still deserialize — mirrors how
   `last_scraped_at` was added.
2. `get_changed_chargers_for_run` SELECT + `row_to_export_changed` — select and map the columns.
3. `save_chargers_from_diff` INSERT (~lines 795–820) — add columns + binds + `ON CONFLICT DO
   UPDATE SET`.
4. `apply_snapshot` INSERT (~lines 947–965) — add columns + binds.
5. `export-snapshot` read path building `ExportChangedCharger` for `coming_soon_superchargers` —
   select the new columns.

### The propagation gap — ✅ **DECIDED (D5): Option A + one-time snapshot**

After D0, `project_status` *is* the status, so its transitions (`Preliminary → Design → Construction`)
are real `status_changes` that flow to prod through the diff automatically. The only fields that can
change **without** a status change are `num_charger_stalls`, `charging_accessibility`, and the address
columns.

**Decision: Option A.**

1. **On deploy, do a one-time full snapshot** to backfill all existing prod rows with the new columns.
   Sequence: deploy + run the migration on both → run **one local scrape** (so local rows have the new
   fields) → `export-snapshot` → `apply_snapshot` on prod → resume normal diff imports.
2. **Afterwards, accept the self-healing lag** on the three leftover fields. When a charger gets its
   next status change, the diff's `changed_chargers` carries its **full current record**, so any stale
   stalls/accessibility/address are corrected then. Coming-soon chargers progress through stages over
   their lifecycle, so values catch up at the next transition — temporary lag, never permanent loss.

Revisit **B** (treat those three as change triggers in `compute_sync` + broaden the diff query) only if
real-time exactness between status changes is ever needed. **C** (periodic snapshot) is unnecessary
given the self-healing.

---

## Decisions (resolved)

Consolidated decision log. All items resolved — see `docs/open-decisions.html` for full context.

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **D0** | **Status-model integration** (see [Status-model integration](#status-model-integration---decided-d0-re-model-around-project_status-stored-as-text)) | 1 add+audit; 2 add column only; 3 full re-model | ✅ **DECIDED: Option 3** — re-model around `project_status`, stored as **TEXT**, derived on scan, single vocabulary. Sub-decision (day-one re-classification): **(a) accept the burst.** |
| D1 | How to store enum-like fields | (a) `String` + const known-set + warn; (b) Rust enum with `Other(String)` fallback | ✅ **DECIDED: (a)** — plain `String` + `KNOWN_*` + warn; `SiteStatus` enum for derived `status` only |
| D2 | Surface unrecognised-value counts beyond logs (e.g. in `status` CLI / scrape-run summary) | yes / no | ✅ **DECIDED: No for v1** — logs only |
| D3 | Persist the unrecognised-value summary on `scrape_runs` (e.g. `JSONB` column) | yes / no | ✅ **DECIDED: No for v1** |
| D4 | Derive `city`/`region` from `functions[0].address` instead of title parsing | now / later / never | ✅ **DECIDED: Later** — keep title parsing for v1; store structured address columns only |
| D5 | Import/export propagation gap | A accept; B change-triggers; C periodic snapshot | ✅ **DECIDED: A** — one-time snapshot on deploy (backfill); status flows via diff (D0); leftover fields (stalls/accessibility/address) lag only between status changes and self-heal at the next one. |
| D6 | Capture `real_estate_status` (`functions[0].status`) at all? | defer / capture (Rule A) / capture + maybe rank | ✅ **DECIDED: defer from v1** — decoupled from build, confusing, no consumer; trivial to add later. Address fields still kept. |
| D7 | Ratchet `real_estate_status` (Rule B) | enable / leave off | ✅ **DECIDED: moot** — nothing captured to ratchet (D6). No Rule B in v1. |
| D8 | Status regression policy (`project_status` / derived `status`) | full ratchet / record all / hybrid | ✅ **DECIDED: hybrid** — forward always; no regress on `UNKNOWN` or coarse-fallback alone; explicit backward `project_status` recorded + `warn!` |
| D9 | Ratchet `site_status` / our `SiteStatus` | ratchet / record | ✅ **Moot** — audited `status` is D8's field; Tesla `site_status` not stored (D10). Never ratchet. |
| D10 | Store raw `site_status` as its own column | yes / no | ✅ **DECIDED: No** — redundant (coarser than `customer_facing`; coming_soon/open already in the lifecycle). May still help open-detection later (separate). |
| D11 | Capture `installed_full_power` at graduation into `opened_superchargers` | now / future / never | ✅ **DECIDED: Now** — `installed_full_power_kw INTEGER` on `opened_superchargers`; from open-check at graduation |

## Testing

- `compute_sync` unit tests: update the `charger(...)` helper; assert new fields survive into
  `upserts` / `unchanged`.
- A parse test for the `0 = unknown` mapping (missing, `"0"`, and `"8"` → `0`, `0`, `8`).
- A `validate_*` test asserting an unknown value logs a warning but is still stored verbatim.
- Repository round-trip: save a charger with the fields, read back via `get_coming_soon`.
- Manual: `cargo run -- scrape --show-browser`, then `cargo run -- status` / hit the API and confirm
  fields populate for known sites (e.g. slug `438058` → 8 stalls, `Design`, address `5 Via Sergio
  Fraccalanza`, `IT`).
- Open-check parse test: `installed_full_power` `"250"` → `Some(250)`; `"0"` / missing → `None`.
- `cargo fmt` + `cargo clippy` before committing.

## Work breakdown & PR plan

Three backend PRs, ordered so the **low-risk additive work lands first** and the **breaking status
re-model is isolated** and shipped with the frontend. PR3 is independent and can go anytime.

### PR 1 — Capture new detail fields (additive, no status change) · risk: low

Delivers stalls / accessibility / address / raw project_status **without touching the status model**.
The existing `IN_DEVELOPMENT` / `UNDER_CONSTRUCTION` status keeps working unchanged.

- **Fetch:** query string → `coming_soon_supercharger,supercharger`; raw types (`RawSuperchargerFunction`,
  `RawFunction`, `RawAddress`); merge into `ComingSoonDetails`.
- **Migration (additive only):** `ADD COLUMN raw_project_status, num_charger_stalls NOT NULL DEFAULT 0,
  charging_accessibility, street_address, county, postal_code, country_code`. No enum change.
- **Domain:** new fields on `ComingSoonSupercharger`; `0 = unknown` stall parsing.
- **Warn-on-unknown infra:** `UnknownEnumTracker` (deduped warn + end-of-run summary), `KNOWN_*` for
  `charging_accessibility` and raw `project_status`.
- **Persistence:** write new columns in both `save_chargers` paths + **Rule A** guards.
- **API:** **unchanged** — public API responses are not touched in PR1 (field exposure is deferred to
  PR2 so all API/contract changes ship once, with the frontend). Internal struct-building reads
  (`get_failed_*`) still select the new columns.
- **Import/export:** new fields on `ExportChangedCharger`; `save_chargers_from_diff`, `apply_snapshot`,
  export read paths (so prod is backfilled before PR2 exposes the fields).
- **Tests:** stall parsing, Rule A, warn-on-unknown, repo round-trip.
- **Deps:** none. **FE coordination:** none (purely additive). Ships independently.

### PR 2 — Re-model status around `project_status` (D0 + D8) · risk: high · **coordinate with FE**

The breaking change. Isolated so review + the frontend deploy + the snapshot backfill all focus here.

- **Domain:** rework `SiteStatus` → `PRELIMINARY/DESIGN/CONSTRUCTION/OPENED/UNKNOWN/REMOVED`; drop the
  `#[sqlx(type_name)]` enum mapping (String↔enum via `Display`/`FromStr`); `derive_status`;
  `resolve_status_transition` (D8 policy); wire into `compute_sync`.
- **Migration (enum → TEXT):** convert `coming_soon_superchargers.status` and `status_changes.old/new`
  to TEXT with the remap `CASE` (live data + history in one pass); `DROP TYPE site_status`.
- **API (all contract changes land here):** status filter param values + `DbStats` /
  `count_coming_soon_by_status` buckets → `PRELIMINARY/DESIGN/CONSTRUCTION`; **plus** expose the T1
  additive fields on `ApiSupercharger` + SELECTs (deferred from PR1); `docs/API.md`.
- **Tests:** `derive_status`, `resolve_status_transition` (all D8 rows), migration remap, day-one burst
  behaviour.
- **Deps:** PR1 (needs `raw_project_status` + `project_status` in `ComingSoonDetails`).
- **Deploy:** ship with the FE PR; expect the day-one `PRELIMINARY → DESIGN` burst (accepted, D0a); do
  the one-time snapshot (below).

### PR 3 — `installed_full_power` at graduation (D11) · risk: low · independent

- **Migration:** `ADD COLUMN installed_full_power_kw INTEGER` on `opened_superchargers`.
- **Fetch/domain:** `installed_full_power` on `OpenCheckSuperchargerFunction`; parse (`"0"`/missing →
  `None`); `installed_full_power_kw` on `OpenResult`.
- **Persistence + import/export:** graduation INSERT, `save_chargers_from_diff`, `apply_snapshot`,
  `ExportOpenedCharger`, opened read paths.
- **Tests:** open-check parse.
- **Deps:** none (orthogonal). Could fold into PR1 if you prefer fewer PRs.

### Deploy runbook (one snapshot total, after PR2)

1. Ship **PR1** (and **PR3**) — additive, safe, no FE dependency. Prod's new columns arrive NULL/`0`
   and self-heal via diffs; no snapshot strictly required yet.
2. Ship **PR2** together with the frontend PR (status contract changes simultaneously).
3. Migrations auto-run on startup (local + prod).
4. Run **one local scrape** → populates all new fields and triggers the `PRELIMINARY → DESIGN` re-classification.
5. **`export-snapshot` → `apply_snapshot`** on prod — single full backfill of every row with all new fields.
6. Resume normal diff imports.

## Frontend tasks (separate repo — rough spec, refine there)

Lands **with PR2** (status contract change). Items below are additive unless noted.

- **[Breaking] Status value set.** Replace `IN_DEVELOPMENT` / `UNDER_CONSTRUCTION` everywhere with the
  new ALL-CAPS values: `PRELIMINARY`, `DESIGN`, `CONSTRUCTION` (+ `OPENED` / `UNKNOWN` / `REMOVED`).
  Touches: status filter dropdown/chips, labels/i18n strings, legend, and any status→colour mapping
  (Preliminary and Design were both "In Development" before — pick distinct colours/labels).
- **Status counts / stats.** Update any dashboard counts that bucketed `in_development` /
  `under_construction` to the new buckets.
- **Detail view — new fields.** Show `num_charger_stalls` (treat **`0` as "unknown"** — hide or show
  "—", not "0 stalls"), `charging_accessibility`, and the structured address
  (`street_address`, `county`, `postal_code`, `country_code`). Optionally show raw `project_status`.
- **Opened chargers.** Show `installed_full_power_kw` as e.g. "250 kW" when present (may be null on
  pre-existing opened rows — render only when set).
- **Recent-changes feed (heads-up).** On deploy day there's a one-time spike of `PRELIMINARY → DESIGN`
  events (discovery, not real moves). Optional: a subtle note, or just let it scroll. No code needed
  unless you want to dedupe/annotate.
- **Map/list styling.** Any status-driven marker/row styling updated to the new value set.
