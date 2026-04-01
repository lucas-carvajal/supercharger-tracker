# Plan: Upload Local Scrape Results to Remote Server

## Background

The scraper bypasses Akamai Bot Manager via headless Chrome (CDP). In practice, after a handful
of requests the IP gets blocked, requiring VPN rotation between runs. Until VPN rotation is
automated, scrapes run locally and the results need to be pushed to the server's PostgreSQL DB.

The server already runs the read-only HTTP API and DB. This document evaluates all sensible ways
to bridge the gap between a local scrape and a remote DB update.

---

## Why the Existing `--file` Mode Is Not Suitable for Uploads

The current `--file` flag (`load_from_file()` in `src/loaders.rs`) reads the raw Tesla API JSON
format (`ApiResponse { data: { data: Vec<Location> } }`). It explicitly sets:

```rust
coming_soon_details: HashMap::new(),
failed_detail_ids: HashSet::new(),
```

This means **no per-charger details are fetched**, so every charger ends up with
`SiteStatus::Unknown`. All real status data (`IN_DEVELOPMENT`, `UNDER_CONSTRUCTION`) is lost.
This format is fine for offline development and replay testing, but is unsuitable for production
uploads where accurate statuses are the whole point.

---

## Upload Format

### Rejected: Raw Tesla API JSON

The existing `ApiResponse` format. Discarded for the reason above — all statuses become UNKNOWN.

### Rejected: Pre-computed `SyncPlan`

Computing the diff locally and uploading just the delta sounds efficient, but the local machine
has a different (or nonexistent) DB state than the server. `compute_sync()` must run against the
*server's* current DB state to produce a correct diff. Uploading a locally-computed plan would
apply the wrong diff.

### Recommended: `ScrapeExport` JSON Envelope

Serialize the already-processed `Vec<ComingSoonSupercharger>` — the output after detail fetching
— wrapped in a thin metadata envelope:

```json
{
  "format_version": 1,
  "scraped_at": "2026-04-01T10:00:00Z",
  "country": "US",
  "run_type": "full",
  "failed_detail_ids": ["12345"],
  "chargers": [
    {
      "id": "11255",
      "title": "Austin, Texas",
      "city": "Austin",
      "region": "Texas",
      "latitude": 30.2672,
      "longitude": -97.7431,
      "status": "IN_DEVELOPMENT",
      "raw_status_value": "In Development"
    }
  ]
}
```

**Why this works well:**

- `ComingSoonSupercharger` already derives `Serialize`/`Deserialize` — no additional code needed.
- Carries real status data from the per-charger details fetch.
- `failed_detail_ids` is preserved so `compute_sync()` on the server can correctly apply the
  "preserve old status when details fetch failed" logic.
- `format_version` guards against schema drift as the type evolves.
- `scraped_at` records when the scrape actually ran, not when it was imported — keeps
  `scrape_runs` provenance accurate.
- Human-readable; inspectable with `jq` before applying.
- Small (~200–400 KB for ~600 worldwide chargers).
- Fully decoupled from DB schema — no SQL types in the wire format.

---

## Upload Mechanism Options

### Option 1 — SSH Tunnel to PostgreSQL

```bash
ssh -L 5432:localhost:5432 user@server
DATABASE_URL=postgres://postgres:pass@localhost:5432/supercharger-db cargo run -- scrape
```

The full existing pipeline runs locally against the remote DB via the forwarded port.

| | |
|---|---|
| **Pros** | Zero code changes. Uses the complete existing pipeline. |
| **Cons** | Requires SSH access and careful DB auth config. Tunnel drop mid-transaction rolls back cleanly (sqlx transaction), but is operationally fragile. Exposes the DB port briefly over SSH. Hard to automate. |
| **Code changes** | None |
| **Security** | Acceptable if Postgres listens only on localhost and SSH is tightly controlled. Postgres scram-sha-256 auth provides a second factor. |

### Option 2 — Export file + `import` CLI subcommand ✅ Recommended (Phase 1)

Add `--export <path>` to the `scrape` subcommand and a new `import` subcommand. The import
handler reuses `record_scrape_run` → `get_current_statuses` → `compute_sync` → `save_chargers`
identically to a live scrape run.

```bash
# 1. Scrape locally, write export file instead of writing to DB
cargo run -- scrape --export /tmp/scrape-2026-04-01.json --export-only

# 2. Transfer by any mechanism (SCP, rsync, S3, USB, etc.)
scp /tmp/scrape-2026-04-01.json user@server:/tmp/

# 3a. Import on the server
DATABASE_URL=postgres://... cargo run -- import --file /tmp/scrape-2026-04-01.json

# 3b. OR import locally, pointing DATABASE_URL at the remote DB directly
DATABASE_URL=postgres://server:5432/supercharger-db cargo run -- import --file /tmp/scrape-2026-04-01.json
```

| | |
|---|---|
| **Pros** | Auditable artefact — inspect with `jq` before applying. Idempotent re-import (upsert semantics). No new network attack surface. No auth design needed. All existing DB functions reused unchanged. |
| **Cons** | Three manual steps per scrape cycle. Still requires either SSH or direct DB access to apply the import. |
| **Code changes** | New `src/export.rs`, `--export`/`--export-only` flags on `scrape`, new `import` subcommand and `run_import()` handler in `main.rs`. ~50–80 lines total. |
| **Security** | No new surface. The JSON file is low-sensitivity (location data, no PII). |

### Option 3 — `POST /scrape-uploads` API endpoint + `upload` CLI subcommand (Phase 2)

Add a write endpoint to the Axum server. The local machine POSTs the `ScrapeExport` JSON with a
bearer token. The server runs the import pipeline.

```bash
cargo run -- upload --server https://api.example.com --file /tmp/scrape-2026-04-01.json
# reads UPLOAD_TOKEN from env, or pass --token
```

| | |
|---|---|
| **Pros** | Fully automated end-to-end — one command, no SSH, no SCP. Works behind NAT (HTTPS outbound only). Composable with cron jobs or CI. |
| **Cons** | Adds write capability + auth to a currently auth-free public API. Token must be kept secret on every machine that uploads. Requires HTTPS in front of the server for token safety in transit. |
| **Code changes** | New `src/api/auth.rs` (bearer token extractor), `src/api/uploads.rs` (handler), route in `api/mod.rs`, `Command::Upload` subcommand in `main.rs`. |
| **Security** | Static bearer token via `UPLOAD_TOKEN` env var. 404 response on auth failure (avoids confirming endpoint existence). Endpoint is write-only — cannot read DB data. |

---

## Recommended Phased Approach

### Phase 1 — Export flag + Import subcommand

Lowest risk, fastest to implement. Gives the team an auditable artefact and a clean import path
without opening any new network attack surface.

**New type in `src/export.rs`:**

```rust
pub const FORMAT_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
pub struct ScrapeExport {
    pub format_version: u8,
    pub scraped_at: DateTime<Utc>,
    pub country: String,
    pub run_type: String,                 // "full" | "retry"
    pub chargers: Vec<ComingSoonSupercharger>,
    pub failed_detail_ids: Vec<String>,   // HashSet in memory, Vec in JSON
}
```

**`run_scrape()` change** (after building `coming_soon`, before DB calls):

```rust
if let Some(ref export_path) = export {
    let payload = ScrapeExport {
        format_version: FORMAT_VERSION,
        scraped_at: Utc::now(),
        country: country.clone(),
        run_type: "full".to_string(),
        chargers: coming_soon.clone(),
        failed_detail_ids: result.failed_detail_ids.iter().cloned().collect(),
    };
    fs::write(export_path, serde_json::to_string_pretty(&payload)?)?;
    println!("Exported {} chargers to {export_path}", coming_soon.len());
    if export_only { return Ok(()); }
}
```

**`run_import()` handler** (reuses all existing functions — no new DB code):

```rust
async fn run_import(pool: &PgPool, file: &str) -> Result<(), Box<dyn Error>> {
    let export: ScrapeExport = serde_json::from_str(&fs::read_to_string(file)?)?;

    if export.format_version != FORMAT_VERSION {
        return Err(format!("unsupported format version {}", export.format_version).into());
    }

    let failed_ids: HashSet<String> = export.failed_detail_ids.into_iter().collect();

    let run_id = db::record_scrape_run(
        pool, &export.country, export.chargers.len() as i32,
        failed_ids.len() as i32, "upload",
    ).await?;

    let current = db::get_current_statuses(pool).await?;
    let plan = sync::compute_sync(current, &export.chargers, &failed_ids);

    db::save_chargers(
        pool,
        &plan.upserts, &plan.unchanged, &plan.status_changes,
        &plan.disappeared_ids, run_id, &failed_ids,
    ).await?;

    println!(
        "Import complete: {} upserted, {} status changes, {} disappeared, {} unchanged",
        plan.upserts.len(), plan.status_changes.len(),
        plan.disappeared_ids.len(), plan.unchanged.len(),
    );
    Ok(())
}
```

No DB schema changes needed. `scrape_runs.run_type` is already `TEXT`; inserting `"upload"`
requires no migration.

### Phase 2 — HTTP Upload Endpoint

Once Phase 1 is proven and HTTPS is configured in front of the server, add:

- `src/api/auth.rs` — `UploadAuth` Axum extractor checking `Authorization: Bearer <token>`
  against `UPLOAD_TOKEN` env var
- `src/api/uploads.rs` — `upload_handler` that deserializes `ScrapeExport` and runs the same
  pipeline as `run_import()`
- Route: `POST /scrape-uploads` in `api/mod.rs`
- `Command::Upload` subcommand with `--server`, `--file`, `--token` / `UPLOAD_TOKEN` env var
- `.env.example`: add `UPLOAD_TOKEN=`

The wire format is identical to Phase 1 export files — fully forward-compatible.

---

## Files Affected

| File | Change |
|---|---|
| `src/export.rs` | **New** — `ScrapeExport` struct, `FORMAT_VERSION` |
| `src/main.rs` | `--export`/`--export-only` on `scrape`; `Command::Import`; `run_import()`; dispatch arm |
| `src/db.rs` | No changes — `record_scrape_run`, `get_current_statuses`, `save_chargers` reused as-is |
| `src/coming_soon.rs` | No changes — `ComingSoonSupercharger` already has `Serialize`/`Deserialize` |
| `src/sync.rs` | No changes — `compute_sync()` reused as-is |
| `src/api/mod.rs` | Phase 2 only: route + `mod uploads` |
| `src/api/auth.rs` | Phase 2 only: bearer token extractor |
| `src/api/uploads.rs` | Phase 2 only: upload handler |
| `.env.example` | Phase 2 only: `UPLOAD_TOKEN=` |
