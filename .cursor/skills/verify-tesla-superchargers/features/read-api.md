# Read API

The `host` API lets an operator list coming-soon sites, inspect one site and its history, and read scrape-run metadata. Read routes do not require the admin secret.

## Sub-features

- `health` reports database reachability.
- `list` returns active coming-soon sites with pagination.
- `list-status` filters `PRELIMINARY`, `DESIGN`, `CONSTRUCTION`, or `UNKNOWN`.
- `list-region` filters by the `?region=` aliases in `docs/API.md`.
- `list-country` filters by `?country=` ISO-2 (`GB`, `US`). Invalid values are `400`.
- `stats` returns per-status counts and `as_of`.
- `map` returns unpaginated markers.
- `detail` returns one site and `status_history`.
- `detail-missing` is 404 for an unknown or opened id.
- `recent-changes` lists transitions with a non-null `old_status`.
- `recent-additions` lists first-seen rows.
- `recent-updates` lists first-seen rows and transitions except `REMOVED` and `UNKNOWN`.
- `scrape-runs` lists recent runs.

## How to get to it (user POV)

- Run `tesla-superchargers host` and call the routes in `docs/API.md`.
- Open the same routes from the Next.js app against this backend.

## Driving it with verify-tesla-superchargers

Preconditions:

- Seeded launch for `<run_id>`.
- `scripts/doctor.sh <run_id>` is green.

- **Health.** Run `scripts/http.sh <run_id> GET /health --out evidence/<run_id>/read-api/health.json`. HTTP `200` and body `{"status":"ok"}`.
- **List.** Run `scripts/http.sh <run_id> GET /superchargers/soon --out evidence/<run_id>/read-api/list.json`. `total` is `2`. `items` include ids `11255` and `12001`. Neither item has `status` `REMOVED`.
- **Status filter.** Run `scripts/http.sh <run_id> GET /superchargers/soon?status=DESIGN`. `total` is `1` and the only id is `11255`.
- **Bad status.** Run `scripts/http.sh <run_id> GET /superchargers/soon?status=OPENED`. HTTP `400` with `invalid status`.
- **Region filter.** Run `scripts/http.sh <run_id> GET /superchargers/soon?region=UK`. `total` is `1` and the id is `11255`. Run `scripts/http.sh <run_id> GET /superchargers/soon?region=TX`. `total` is `1` and the id is `12001`.
- **Country filter.** Run `scripts/http.sh <run_id> GET /superchargers/soon?country=GB --out evidence/<run_id>/read-api/list-country-gb.json`. `total` is `1` and the id is `11255`. The item `country` is `GB`. Run `scripts/http.sh <run_id> GET /superchargers/soon?country=US --out evidence/<run_id>/read-api/list-country-us.json`. `total` is `1` and the id is `12001`. The item `country` is `US`.
- **Bad country.** Run `scripts/http.sh <run_id> GET /superchargers/soon?country=UK`. HTTP `200` and `total` is `0`. `UK` is two letters so it is not `400`. It does not alias to `GB`. Run `scripts/http.sh <run_id> GET /superchargers/soon?country=germany`. HTTP `400` with `invalid country`. Run `scripts/http.sh <run_id> GET /superchargers/soon?country=ZZ`. HTTP `200` and `total` is `0`.
- **Stats.** Run `scripts/http.sh <run_id> GET /superchargers/soon/stats`. `total_active` is `2`. `by_status.DESIGN` is `1`. `by_status.CONSTRUCTION` is `1`. `as_of` is `2026-03-31T08:45:00Z`.
- **Map.** Run `scripts/http.sh <run_id> GET /superchargers/soon/map`. A JSON array of length `2` with ids `11255` and `12001`. Each marker has `country` (`GB` for `11255`, `US` for `12001`).
- **Detail.** Run `scripts/http.sh <run_id> GET /superchargers/soon/11255 --out evidence/<run_id>/read-api/detail-11255.json`. `status` is `DESIGN`. `country` is `GB`. `status_history` has a first-seen `PRELIMINARY` row (`old_status` null) and a `PRELIMINARY` to `DESIGN` row. `tesla_url` is `https://www.tesla.com/findus?location=11255`.
- **Opened id.** Run `scripts/http.sh <run_id> GET /superchargers/soon/99999`. HTTP `404`. Opened sites are not on this route.
- **Recent changes.** Run `scripts/http.sh <run_id> GET /superchargers/soon/recent-changes`. `items` include `11255` `PRELIMINARY` to `DESIGN` and `99999` `CONSTRUCTION` to `OPENED`. First-seen rows are absent. `11255` has `country` `GB`. `99999` has `country` `US`.
- **Recent additions.** Run `scripts/http.sh <run_id> GET /superchargers/soon/recent-additions`. Ids include `11255` and `12001`. Both items have `country`.
- **Recent updates.** Run `scripts/http.sh <run_id> GET /superchargers/soon/recent-updates`. Ids include the first-seen rows and the `DESIGN` and `OPENED` transitions. Items include `country`.
- **Scrape runs.** Run `scripts/http.sh <run_id> GET /scrape-runs`. `items[0].id` is `1` and `country` is `US`.
- **Proof.** Keep `list.json` and `detail-11255.json`. Both must identify `11255` and `DESIGN`.

## Gotchas

- List, map, and stats hide `REMOVED` rows. They never show opened sites. `99999` is only in export JSON and `opened_superchargers`.
- `num_charger_stalls: 0` means unknown, not zero stalls.
- `?region=NT` matches both Australian Northern Territory and Canadian Northwest Territories.
- Recent-changes omit first-seen rows (`old_status` null) and omit `new_status = UNKNOWN`.
- Recent-updates omit `REMOVED` and `UNKNOWN`.
- `GET /health` is `503` when Postgres is down. Doctor already fails in that case.
