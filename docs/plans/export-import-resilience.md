# Export / Import Resilience

> Status: research / design notes — not yet implemented.

## 1. Background

`supercharger-tracker` runs in a split-environment topology:

- **Local** — Where scraping happens. Headless Chrome, full DB, the user's day-to-day workspace.
- **Prod** — Hosts the read-only HTTP API. Receives scrape results via `POST /scrapes/import` from local export files.

Today there are two transport formats (`src/export.rs`):

| Format | Producer | Consumer (on prod) | Behaviour |
|---|---|---|---|
| `DiffExport` | `cargo run -- export-diff` | `apply_diff` in `src/application/import.rs` | Strictly sequential. `run_id == MAX(scrape_runs.id) + 1` is enforced unless `--force`. |
| `SnapshotExport` | `cargo run -- export-snapshot` | `apply_snapshot` in `src/repository/supercharger.rs` | Destructive. `TRUNCATE TABLE status_changes, coming_soon_superchargers, opened_superchargers, scrape_runs RESTART IDENTITY CASCADE`, then re-INSERT every row with its original id. |

Both formats hard-couple the local and prod identifier spaces: the local
`scrape_runs.id` is forced into prod via `OVERRIDING SYSTEM VALUE`, and prod's
ordering check (`MAX(id) + 1`) and dedup (`WHERE id = $1`) both depend on that
shared sequence.

## 2. The actual problems

### 2.1. Snapshots wipe prod history

`apply_snapshot` is `TRUNCATE … CASCADE` followed by INSERTs from the file
(`src/repository/supercharger.rs:850-950`). Anything prod has that isn't in the
file is destroyed:

- Prior `status_changes` rows attributed to runs the local DB has since forgotten.
- Manual fixups, comments, or backfills made directly on prod.
- Any divergence from the local DB at all.

This is fine for "fresh prod, first install" but unsafe as a recovery tool once
prod has accumulated its own state.

### 2.2. Local DB reset has no safe recovery path

If you reset (or lose) the local DB:

- A fresh local `scrape` produces `scrape_runs.id = 1`.
- Prod still has `MAX(id) = N` for some large N.
- The next diff fails the ordering check (`expected = N+1, got = 1`).
- `--force` would let it through, but two diffs then collide on `coming_soon_superchargers.id` (the Tesla slugs are the same), and on `status_changes` (no dedup at all).
- `export-snapshot` from the freshly-reset local would TRUNCATE prod, taking all the prior history with it.

Result: there is **no** safe way to recover from a local reset that preserves prod history.

### 2.3. Diff ordering is fragile

Diffs must be applied in strict order. Any of these break the chain:

- A scrape was run and discarded without exporting → run ids advance locally, gap on prod, all subsequent diffs are rejected.
- A diff file is lost or fails to upload mid-way through a session.
- Two devs (or two machines) scrape locally and try to push.
- A user runs `scrape` with the wrong country flag, exports, and wants to throw it away.

`--force` exists but is footgun-flavoured: it bypasses ordering but doesn't
protect against the underlying issues (id collisions, duplicate
`status_changes` entries, etc.).

### 2.4. `OVERRIDING SYSTEM VALUE` is the smell

Prod and local are separate Postgres instances but share a `BIGSERIAL` id space
by force. Every other problem in this list flows from that decision:

- Snapshots have to TRUNCATE because they re-insert with explicit ids.
- Diffs have to enforce ordering because the id is being used as both a primary key and a logical sequence number.
- Resetting one side desyncs both.

A serial id should be private to its database.

## 3. Goals for a redesign

1. **Decouple local and prod identifiers.** Prod's `scrape_runs.id` should be assigned by prod. Local's should be assigned by local. They should never collide because they're never compared.
2. **Make snapshots additive.** A snapshot should be a "merge this state in" operation, not "throw out everything and restart". Destructive replace can stay as an opt-in.
3. **Make diffs idempotent and order-independent.** Replaying the same diff twice is a no-op. Applying diffs out of order produces the same end state as applying them in order (within reason — see §5.4).
4. **Survive local DB resets.** A user should be able to `DROP DATABASE; cargo run -- scrape; cargo run -- export-diff` and have prod accept the result without losing history.
5. **Preserve causal history.** The `status_changes` audit log is the most valuable artefact in the system. It must never be destroyed by an import, never be silently deduped wrong, and never be reordered.

## 4. Design options

The options below are not mutually exclusive — several should be combined. They are presented from "minimal change" to "structural redesign".

### Option A — Additive snapshot mode (smallest change)

Add a `--mode=merge` (default) and `--mode=replace` (current behaviour) to
snapshot import. In merge mode:

- `coming_soon_superchargers` and `opened_superchargers` use `INSERT … ON CONFLICT (id) DO UPDATE SET …` instead of TRUNCATE+INSERT. Last-write-wins on the row body, preserving prod's `first_seen_at` if the prod row is older.
- `scrape_runs` rows are INSERTed with `ON CONFLICT (id) DO NOTHING` — but this still requires id alignment, so this option alone is only a partial fix.
- `status_changes` needs a dedup key (see Option D) to avoid being inserted twice.

**Pros:** very small code change; immediately removes the worst footgun (snapshot
wipes prod).
**Cons:** doesn't fix any of the fundamental id-coupling issues. The dedup
problem on `status_changes` is unsolved without Option D.

### Option B — Decouple ids via `source_run_uuid`

Add a `source_run_uuid UUID NOT NULL` column to `scrape_runs`. Every scrape
generates one locally (e.g. UUIDv7 so it's also roughly time-ordered). Imports
dedupe and reference by `source_run_uuid`, never by `id`.

**Schema:**

```sql
ALTER TABLE scrape_runs ADD COLUMN source_run_uuid UUID;
UPDATE scrape_runs SET source_run_uuid = gen_random_uuid() WHERE source_run_uuid IS NULL;
ALTER TABLE scrape_runs ALTER COLUMN source_run_uuid SET NOT NULL;
ALTER TABLE scrape_runs ADD CONSTRAINT scrape_runs_source_run_uuid_key UNIQUE (source_run_uuid);

ALTER TABLE status_changes ADD COLUMN source_run_uuid UUID;
UPDATE status_changes sc
   SET source_run_uuid = sr.source_run_uuid
  FROM scrape_runs sr
 WHERE sc.scrape_run_id = sr.id;
ALTER TABLE status_changes ALTER COLUMN source_run_uuid SET NOT NULL;
```

**Import path:**

- `apply_diff` no longer uses `OVERRIDING SYSTEM VALUE`. It does a normal INSERT into `scrape_runs` and lets prod assign its own `id`. Dedup is `WHERE source_run_uuid = $1`.
- The local `scrape_runs.id` is still in the export, but only as a hint / display value, not as a key.
- `status_changes.scrape_run_id` is rewritten to point at prod's freshly-assigned id.

**Pros:** structural fix. Local and prod can have wildly different id sequences and never collide. Resetting local doesn't break anything. No more `OVERRIDING SYSTEM VALUE`.
**Cons:** schema migration (cheap). Need to thread `source_run_uuid` through several places.

### Option C — Drop the strict ordering check; enforce per-row monotonicity instead

Replace `MAX(id) + 1` with: **each charger row is updated only if the incoming
`scraped_at` is newer than the current `last_scraped_at`**.

```sql
INSERT INTO coming_soon_superchargers (...)
VALUES (...)
ON CONFLICT (id) DO UPDATE SET
    title           = EXCLUDED.title,
    -- ... etc
    last_scraped_at = EXCLUDED.last_scraped_at
WHERE EXCLUDED.last_scraped_at > coming_soon_superchargers.last_scraped_at;
```

This makes each diff self-contained and idempotent. Importing the same diff
twice is a no-op. Importing diffs A→B→C produces the same per-charger state as
B→A→C, **for the row body**. The history (`status_changes`) is unaffected by
order because changes are timestamped events.

**Pros:** removes ordering fragility entirely. Replays are safe. Out-of-order is safe. Lost diffs become "missing history" rather than "broken pipeline".
**Cons:** 
- Per-charger last-write-wins isn't quite the same as per-run atomicity. If you scrape twice in quick succession on two machines, you get a merged view rather than "machine A's run". For analytics this is fine; flag it.
- Need Option D to make `status_changes` idempotent on replay.

### Option D — Content-addressable `status_changes` dedup key

Add a deterministic dedup key to `status_changes` so re-imports are no-ops.
Two flavours:

**D1: natural-key uniqueness**
```sql
ALTER TABLE status_changes
  ADD CONSTRAINT status_changes_natural_key
  UNIQUE (supercharger_id, source_run_uuid, new_status);
```
Then every insert becomes `INSERT … ON CONFLICT DO NOTHING`.

**D2: content hash**
```sql
ALTER TABLE status_changes ADD COLUMN content_hash BYTEA;
-- hash = SHA256(supercharger_id || source_run_uuid || old_status || new_status || changed_at)
ALTER TABLE status_changes ADD CONSTRAINT status_changes_content_hash_key UNIQUE (content_hash);
```

D1 is simpler and almost always correct. D2 handles the edge case where prod
and local disagree on the *content* of a status change for the same logical event
(shouldn't happen, but defensive).

**Pros:** the audit log becomes idempotent. Combined with B and C, this means imports are fully replayable.
**Cons:** schema migration (cheap). Need to backfill the dedup key for existing rows.

### Option E — Snapshots become "merge by default", with an explicit `--replace` escape hatch

Once Options A + B + D are in place, a snapshot is just "a really big idempotent
diff". The whole snapshot/diff distinction can collapse into a single import
path with `mode = full | partial`. The only thing `mode = full` adds is a
guarantee that *if* prod has a charger that isn't in the snapshot, it can be
removed. Even that should require an explicit flag like `--prune-missing` so
silent data loss is impossible.

**Pros:** simpler model. Fewer subcommands. Snapshots stop being scary.
**Cons:** none, once the prereqs are in place. This is the destination, not a
standalone option.

### Option F — Push directly via API, skip the file dance

Add a `--push` flag to `scrape` (or a new `push` subcommand) that POSTs
the export straight to prod. Combined with Options B–D this becomes the
canonical workflow:

```bash
cargo run -- scrape --push https://api.example.com
```

**Pros:** removes a manual step. Removes the `scrape_export_NNN.json` file
sprawl. If prod rejects it (auth, schema mismatch), you find out immediately.
**Cons:** requires network connectivity from the scrape host. Doesn't replace
file-based exports for offline / debugging use.

### Option G — Per-charger version vectors instead of run-level versioning

Maximalist option: each charger row has its own version counter, bumped on
every change. Imports merge per-row using the version. This is essentially CRDT
territory.

**Pros:** mathematically clean conflict resolution. Survives any topology.
**Cons:** big complexity jump for a use case (single-writer, single-reader)
that doesn't need it. Mentioned for completeness; not recommended.

### Option H — Logical replication (Postgres native)

Set up Postgres logical replication: local as publisher, prod as subscriber.
Built-in, robust, handles all the messy parts.

**Pros:** zero application-level code. Battle-tested.
**Cons:**
- Requires direct Postgres connectivity from local to prod (or a tunnel). The current architecture relies on prod being only HTTP-exposed.
- Schema migrations on prod become tricky.
- Doesn't handle the "I want to discard a local scrape without it ever touching prod" use case.
- Forces 1:1 mirroring; no notion of "partial export".

Probably overkill, but worth knowing it exists.

## 5. Recommended path

Combine **B + C + D + E**, rolled out in stages so each stage is independently
useful and each migration is reversible.

### Stage 1 — Schema groundwork (one migration)

```sql
-- decouple identifiers
ALTER TABLE scrape_runs ADD COLUMN source_run_uuid UUID;
UPDATE scrape_runs SET source_run_uuid = gen_random_uuid() WHERE source_run_uuid IS NULL;
ALTER TABLE scrape_runs ALTER COLUMN source_run_uuid SET NOT NULL;
ALTER TABLE scrape_runs ADD CONSTRAINT scrape_runs_source_run_uuid_key UNIQUE (source_run_uuid);

-- status_changes carries the source uuid too
ALTER TABLE status_changes ADD COLUMN source_run_uuid UUID;
UPDATE status_changes sc
   SET source_run_uuid = sr.source_run_uuid
  FROM scrape_runs sr
 WHERE sc.scrape_run_id = sr.id;
ALTER TABLE status_changes ALTER COLUMN source_run_uuid SET NOT NULL;

-- idempotent re-imports
ALTER TABLE status_changes
  ADD CONSTRAINT status_changes_natural_key
  UNIQUE (supercharger_id, source_run_uuid, new_status);
```

Local code keeps generating ids the way it does now; only the new column is
added. `record_run` in `src/repository/scrape_run.rs` starts emitting a UUID
at scrape time.

### Stage 2 — Update the export format

`DiffExport` and `SnapshotExport` start carrying `source_run_uuid`. Old fields
stay in place for one release for backwards compatibility (mark
`#[serde(default)]` so older files still parse). The old `run_id: i64` becomes
display-only metadata.

### Stage 3 — Rewrite `apply_diff`

```text
1. Dedup: WHERE source_run_uuid = $1 (was: WHERE id = $1)
2. INSERT scrape_runs without OVERRIDING SYSTEM VALUE; capture the new prod id.
3. Upsert chargers, gated on EXCLUDED.last_scraped_at > current.last_scraped_at.
4. Insert status_changes with the new prod scrape_run_id, ON CONFLICT (natural key) DO NOTHING.
5. opened_superchargers as today: ON CONFLICT (id) DO NOTHING.
6. Tombstone removed_ids only if their current status is older than the diff.
```

The ordering check is removed entirely. The `force` flag goes away (or
becomes purely advisory).

### Stage 4 — Rewrite `apply_snapshot` as merge

`apply_snapshot` becomes structurally identical to `apply_diff`, just over a
larger payload. The TRUNCATE goes away. The current destructive behaviour
moves behind an explicit `--mode=replace` flag, gated on a confirmation prompt
or a separate endpoint, because it's irreversible.

### Stage 5 — Collapse export formats (optional cleanup)

Once snapshots and diffs share an import path, the `ScrapeExport` enum can
collapse into a single struct with an `is_full: bool` (or just always be
"snapshot-shaped") and the two CLI subcommands become one with a `--scope`
flag. This is purely cosmetic and can wait.

### Stage 6 — Add `scrape --push` (Option F)

Now that imports are safe to retry, a direct push from `scrape` is low-risk.
Wraps the existing `POST /scrapes/import` call in a client. Files remain
available as a fallback / debugging tool.

## 6. Trade-offs and things to watch

- **Atomic-run semantics get fuzzier.** Right now a `scrape_runs` row corresponds 1:1 to a moment in time on prod. After the redesign, prod's `scrape_runs` row records "when this import was applied", not "the totality of changes that batch produced", because some of those changes might be skipped as stale. Mitigation: keep the import payload's `scraped_at` and also record `imported_at` separately. The audit log (`status_changes`) is still complete and ordered.
- **Per-row last-write-wins can lose information in pathological cases.** If two scrapes run in parallel from two machines and produce contradictory data for the same charger, the later `scraped_at` wins per-row, which may produce a Frankenstein. In practice we have one scraper, so this is theoretical. Document it; don't engineer around it.
- **Backfilling `source_run_uuid` for existing rows uses random UUIDs.** That's fine — they're stable from then on. The historical correlation between local and prod ids is lost, but no one was relying on it.
- **Status_changes natural-key dedup assumes `(supercharger_id, source_run_uuid, new_status)` is unique within a single scrape.** It is in the current data model (one row per status transition per charger per run), but if that ever changes — e.g. a charger flips A→B→A within the same run — the constraint would reject the second event. Easy to relax to `(supercharger_id, source_run_uuid, new_status, changed_at)` if needed.
- **Old export files become unreadable after the format change.** Solve with a one-release deprecation window where both formats parse.
- **Replay ordering for `status_changes` displayed via the API.** Currently uses `changed_at DESC`. With out-of-order imports, two events in the same import will have the same `changed_at` (the scrape's `scraped_at`). Add `id` as a tiebreaker — already done in some queries, audit the rest.

## 7. Local-DB-reset workflow after the redesign

```bash
# 1. Local DB is gone. Start fresh.
dropdb supercharger-db && createdb supercharger-db

# 2. Run a normal scrape. Generates fresh local ids and a fresh source_run_uuid.
cargo run -- scrape

# 3. Export a snapshot of local state (it's small — one scrape's worth).
cargo run -- export-snapshot --file fresh.json

# 4. Push to prod. Default mode is merge. Prod upserts the chargers it
#    doesn't have, refreshes the ones it does (only where local's scraped_at
#    is newer), and appends any status_changes it's never seen before
#    (deduped by source_run_uuid). Prod's pre-existing history is preserved.
curl -X POST -H "X-Import-Token: ..." \
     -H "Content-Type: application/json" \
     --data @fresh.json \
     https://api.example.com/scrapes/import

# 5. Subsequent diffs flow normally. No ordering conflicts. No --force needed.
```

If the user truly wants to wipe prod and start over (e.g. after a schema
migration), the explicit `--mode=replace` (or a separate `RESET` operation)
is still available, but it's no longer the default and no longer the only path.

## 8. Open questions

- Should prod store the source `scraped_at` per row or per scrape? Per-row is needed for the merge gate; per-scrape is needed for "show me what scrape introduced this charger" — both are useful, both are cheap.
- Should we also track which prod `scrape_runs.id` was the *first* to introduce a charger (currently `first_seen_at` is a timestamp, no run reference)? Probably not worth it.
- Do we want to expose any of this via the API? E.g. `GET /scrapes` could include the `source_run_uuid` so external clients can correlate with their own ingestion logs. Low priority.
- Should `scrape --push` be authenticated by something stronger than a shared bearer token (e.g. mTLS, JWT)? Out of scope for this plan but worth noting.

## 9. What this plan deliberately does not do

- Doesn't migrate to a real replication topology (Option H). Too heavy for the actual data volume.
- Doesn't introduce per-charger CRDT versioning (Option G). Overkill for single-writer.
- Doesn't change the scraping architecture. Local-scrape, push-to-prod stays.
- Doesn't redesign the API surface. `POST /scrapes/import` stays; only its semantics change.
