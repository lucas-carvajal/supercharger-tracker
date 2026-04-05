-- Export/import pipeline support.
-- Retries now update the parent scrape run row instead of creating a new row.
-- `exported` tracks which local runs have been written to an export file.
-- `source_run_id` (prod-only) tracks the local scrape_runs.id of an imported run;
-- used for dedup and ordering checks on the prod DB.
ALTER TABLE scrape_runs
    ADD COLUMN retry_count   INT     NOT NULL DEFAULT 0,
    ADD COLUMN last_retry_at TIMESTAMPTZ,
    ADD COLUMN exported      BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN source_run_id BIGINT;

CREATE UNIQUE INDEX ON scrape_runs (source_run_id) WHERE source_run_id IS NOT NULL;
