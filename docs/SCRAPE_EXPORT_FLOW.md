# Scrape + Export Flow (Quick Runbook)

This guide is the fastest path to:

1. Run a full scrape
2. Verify the run
3. Export the latest diff JSON
4. Export a full snapshot (when needed)

Use it whenever you need a repeatable local scrape-to-export workflow.

---

## 1) Run a full scrape

```sh
cargo run -- scrape --show-browser
```

Useful variants:

```sh
# Run headless instead
cargo run -- scrape

# Scrape with a specific country code
# Note: US returns worldwide data
cargo run -- scrape --country DE --show-browser
```

What this does:

- Fetches coming-soon sites from Tesla
- Fetches detail/status data per site
- Upserts current data in Postgres
- Records status transitions and scrape run metadata

---

## 2) Verify scrape result

```sh
cargo run -- status
```

Check for:

- Last run timestamp and counts
- Any detail-fetch failures

If there are failures, retry only failed details:

```sh
cargo run -- retry-failed --show-browser
```

Optional:

```sh
# Run headless instead
cargo run -- retry-failed
```

---

## 3) Export the latest scrape diff

Default filename (`scrape_export_{run_id}.json`):

```sh
cargo run -- export-diff
```

Custom filename:

```sh
cargo run -- export-diff --file my_export.json
```

If unresolved detail failures still exist and you intentionally want to export anyway:

```sh
cargo run -- export-diff --force
```

---

## 4) Typical repeatable command sequence

```sh
cargo run -- scrape --show-browser
cargo run -- status
cargo run -- retry-failed --show-browser   # only if status indicates failures
cargo run -- status         # optional re-check
cargo run -- export-diff --file my_export.json
cargo run -- export-diff --file /Users/lucas/Downloads/soonercharger-diff-export-vXX.json
```

---

## 5) Export a full snapshot (full DB state)

Use this when you need a complete baseline export (for example, first-time prod/bootstrap before importing diffs).

```sh
cargo run -- export-snapshot --file /Users/lucas/Downloads/soonercharger-SNAPSHOT-export-vXX.json
```

---

## Notes

- `export-diff` always exports the latest scrape run.
- In first-time prod/bootstrap flows, apply a snapshot before importing diffs.
- This runbook defaults to `--show-browser`; use the headless variants when needed.
