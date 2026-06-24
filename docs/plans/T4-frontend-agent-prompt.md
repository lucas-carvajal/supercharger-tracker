# T4 — Frontend agent prompt

This file is **not a backend ticket** — it's a ready-to-paste prompt to hand to an agent working in the
**frontend repo**. It carries all the context that agent needs about the backend changes. The agent's
job is to **analyze** what the frontend must change (not implement yet).

---

## PROMPT (paste this to the frontend agent)

You are working in the **frontend repo** of "Supercharger Tracker" — a webapp that visualizes Tesla
coming-soon (planned) Supercharger locations and their status, backed by a read-only HTTP/JSON API from
a Rust backend. The backend is being changed in a way that **breaks the status value contract** and
**adds new fields**. Your task is to analyze what needs to change in this frontend — produce a written
analysis and change plan; do **not** implement yet.

### What changed on the backend (context)

The backend now extracts richer data for planned chargers from Tesla's API, and **re-models the status
field** to be finer-grained. Three relevant backend changes shipped:

1. **New descriptive fields** on coming-soon chargers (additive).
2. **Status vocabulary re-model** (breaking) — this is the important one for you.
3. **Installed power on opened chargers** (additive).

### API changes — details

**1. Status values changed (BREAKING).** The `status` field on coming-soon chargers — and the
`status` filter parameter, and the per-status count buckets — used these values **before**:

```
IN_DEVELOPMENT, UNDER_CONSTRUCTION, UNKNOWN, REMOVED, OPENED
```

and now use:

```
PRELIMINARY, DESIGN, CONSTRUCTION, UNKNOWN, REMOVED, OPENED
```

Mapping of the change:
- `IN_DEVELOPMENT` was **split into two finer stages**: `PRELIMINARY` (earliest) and `DESIGN`.
- `UNDER_CONSTRUCTION` → `CONSTRUCTION`.
- `UNKNOWN`, `REMOVED`, `OPENED` are unchanged.

Values are ALL-CAPS / SCREAMING_SNAKE, same casing convention as before. The build pipeline order is
`PRELIMINARY → DESIGN → CONSTRUCTION → (OPENED)`.

**2. New response fields on coming-soon chargers** (all additive; treat as optional/nullable):
- `num_charger_stalls` — integer. **`0` means "unknown / not yet published"**, NOT "zero stalls".
  Render `0` as "—" / "unknown" / hidden, never "0 stalls".
- `charging_accessibility` — string or null. Observed values: `"Tesla Only"`,
  `"All Vehicles (Production)"`, `"NACS Partner Enabled (Production)"`.
- `raw_project_status` — string or null (raw Tesla label, title-case e.g. `"Design"`; informational —
  prefer the canonical `status` field for logic).

> Note: a structured address (street/county/postal/country) is captured by the backend but is **not
> exposed in the API** — do not plan UI for it.

**3. Opened chargers** gained `installed_full_power_kw` — integer or null (e.g. `250`). May be null on
chargers that opened before this change. Render as e.g. "250 kW" only when present.

**4. One-time data event on deploy day:** the "recent changes" feed will show a one-time burst of
`PRELIMINARY → DESIGN` transitions (the backend re-derived existing chargers to the finer stages). These
aren't real-world moves. No action required, but don't be alarmed by the spike.

> Source of truth for exact response shapes/routes: the backend repo's `docs/API.md` (if accessible to
> you) and the **live API responses**. Verify field names and endpoint paths there rather than assuming.

### Your task — ANALYSIS ONLY

Produce a written analysis + change plan covering:

1. **Discover the stack & structure.** Identify the framework, where API types/models live, where
   status is consumed (filters, legends, labels, colors, map markers, list/detail views, charts,
   i18n/translation files), and how status counts/stats are rendered.
2. **Find every usage of the old status values.** Grep for `IN_DEVELOPMENT`, `UNDER_CONSTRUCTION`
   (and any title-case/`"In Development"`/`"Under Construction"` display strings, enums, unions, type
   defs, color maps, sort orders, filter option lists). List each with file:line and what it does.
3. **Plan the breaking migration.** For each usage, what's the change? Note specifically:
   - `IN_DEVELOPMENT` splits into `PRELIMINARY` + `DESIGN` — these need **distinct labels and
     ideally distinct colors/markers** (they were one bucket before). Propose them.
   - `UNDER_CONSTRUCTION` → `CONSTRUCTION` (rename).
   - Any status-count/stat displays that bucketed the old values.
   - Any status-based ordering (the pipeline order is Preliminary → Design → Construction).
4. **Plan the additive UI.** Where/how to surface the new fields: `num_charger_stalls`
   (with the `0 = unknown` rule), `charging_accessibility`, `raw_project_status` (informational), and
   `installed_full_power_kw` on opened chargers. Detail views are the obvious home; call out if any
   belong on map popups / list rows. (Address fields are not available via the API — out of scope.)
5. **Risks & sequencing.** This must deploy **in lockstep with the backend** (the status values flip at
   once — no transition window). Note any place that would break if it receives an unknown status, and
   recommend a tolerant default (e.g. render unrecognized statuses gracefully).

**Deliverable:** a markdown analysis with the inventory (file:line table), the proposed status
label/color mapping for the new 6-value set, the additive-field UI plan, and an ordered task list with
rough effort. Do not change code yet — we'll review the analysis first.
