-- Export/import pipeline support.
-- Retries now update the parent scrape run row instead of creating a new row.
-- `exported` tracks which local runs have been written to an export file.
ALTER TABLE scrape_runs
    ADD COLUMN retry_count   INT     NOT NULL DEFAULT 0,
    ADD COLUMN last_retry_at TIMESTAMPTZ,
    ADD COLUMN exported      BOOLEAN NOT NULL DEFAULT FALSE;
