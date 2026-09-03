# Live scrape

`scrape` and `retry-failed` pull Tesla's Find Us feed through headless Chrome and write the result to Postgres. This verification skill does not drive that path.

## Sub-features

- `scrape-full` fetches worldwide coming-soon sites when `--country US`.
- `scrape-country` passes another country code.
- `retry-failed` retries chargers whose last detail or open-status fetch failed.
- `scrape-browser` shows Chrome when `--show-browser` is set.

## How to get to it (user POV)

- Run `tesla-superchargers scrape`.
- Run `tesla-superchargers scrape --country DE`.
- Run `tesla-superchargers scrape --show-browser`.
- Run `tesla-superchargers retry-failed`.

## Driving it with verify-tesla-superchargers

Preconditions:

- Chrome or Chromium on the machine.
- Network access to `www.tesla.com` that can pass Akamai Bot Manager.
- A disposable database you accept will be written with live Tesla data.

- **Do not drive this feature in the default verification run.** The helpers never start Chrome. Seed `fixtures/snapshot.json` and `fixtures/diff.json` instead.
- **If you must prove scrape later**, run it only on a throwaway database that this skill launched with `--empty`, then use `status` and the read API as the second view. Capture stdout, the new `scrape_runs` row, and a `GET /superchargers/soon/stats` body. Record the Chrome and Tesla prerequisites that were actually met.
- **Proof for the default run.** Write `evidence/<run_id>/scrape/skipped.txt` that names the attempted entry point (`tesla-superchargers scrape`) and the unmet precondition (no Tesla live fetch in this skill).

## Gotchas

- `US` is not a geographic filter. Tesla returns the worldwide set for that value.
- A live scrape can take a long time and can fail under Akamai. Failure is not a product regression in this skill.
- Never point a live scrape at a shared or production `DATABASE_URL`.
- Import and export are the supported way to move scrape results between machines.
