# Tesla Supercharger API — Verified Findings

> All findings below were verified live against `https://www.tesla.com/findus` on 2026-03-12.

---

## Endpoint

```
GET https://www.tesla.com/api/findus/get-locations
```

No authentication required. Public endpoint.

---

## The `country` param is the key

| Request | Total locations | Location types returned |
|---|---|---|
| `/api/findus/get-locations` | **5,071** | `supercharger` only (open) |
| `/api/findus/get-locations?country=US` | **21,450** | All types — including planned |

**Adding `?country=US` unlocks the full dataset**, expanding the response from 5k to 21k+ locations and including all planned/coming-soon entries worldwide (despite the param value being `US`, non-US locations are included).

---

## Getting planned superchargers

Filter the response array on the `location_type` field:

| `location_type` value | Live count | What it means |
|---|---|---|
| `coming_soon_supercharger` | **806** | Planned Superchargers |
| `coming_soon_megacharger` | **63** | Planned Megachargers |
| `coming_soon_service` | **26** | Planned service centres |

Filtering is **client-side only** — the `filters=coming_soon_superchargers` query param seen in the browser URL bar only controls what the map UI renders; the API always returns the full dataset regardless.

---

## Full list of `location_type` values (with `?country=US`)

```
supercharger
coming_soon_supercharger
megacharger
coming_soon_megacharger
destination_charger
destination_charger_nontesla
nacs
service
coming_soon_service
sales
delivery_center
gallery
bodyshop
vendor_collision
self_serve_demo_drive
party
winner_supercharger
current_winner_supercharger
```

---

## Response structure

The response is a JSON object:

```json
{
  "data": {
    "data": [ /* array of location objects */ ]
  }
}
```

### Open Supercharger object

```json
{
  "latitude": 64.250138,
  "longitude": -15.205539,
  "location_type": ["supercharger"],
  "location_url_slug": "Hofnissupercharger",
  "title": "locations",
  "uuid": "19215441",
  "inCN": false,
  "inHkMoTw": false,
  "supercharger_function": {
    "actual_latitude": "64.250138",
    "actual_longitude": "-15.205539",
    "access_type": "Public",
    "open_to_non_tesla": true,
    "project_status": "Open",
    "site_status": "open",
    "charging_accessibility": "All Vehicles (Production)",
    "coming_soon_latitude": "64.250138",
    "coming_soon_longitude": "-15.205464",
    "coming_soon_name": null,
    "show_on_find_us": "1",
    "vote_winner_quarter": null
  }
}
```

### Planned Supercharger object (`coming_soon_supercharger`)

Notably **leaner** — no `supercharger_function` block, just coordinates, title and slug:

```json
{
  "coming_soon_supercharger": true,
  "latitude": 51.22962,
  "longitude": -2.959685,
  "location_type": ["coming_soon_supercharger"],
  "location_url_slug": "11255",
  "title": "Highbridge, United Kingdom",
  "uuid": "11399610"
}
```

---

## How to fetch all planned superchargers (example)

```js
const res = await fetch('https://www.tesla.com/api/findus/get-locations?country=US');
const { data: { data: locations } } = await res.json();

const plannedSuperchargers = locations.filter(
  loc => loc.location_type.includes('coming_soon_supercharger')
);

console.log(`${plannedSuperchargers.length} planned superchargers found`);
// → 806 planned superchargers found
```

---

## Notes

- The `country` param does **not** geographically filter results — it acts as a mode switch that tells the API to return the full multi-type dataset instead of open superchargers only.
- The `filters` URL param on the Find Us page (`?filters=coming_soon_superchargers`) is **UI-only** — it never reaches the API.
- Planned supercharger objects are minimal (no stall count, no open date). More detail may be available via a separate location detail endpoint at `/api/tesla-locations/supercharger/{uuid}`.
