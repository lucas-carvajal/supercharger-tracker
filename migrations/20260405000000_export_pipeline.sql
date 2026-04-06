-- Export/import pipeline support columns on scrape_runs.
--
-- `retry_count` and `last_retry_at`: the `retry-failed` command no longer
-- creates a new scrape_runs row for each retry pass. Instead it UPDATEs these
-- fields on the original scrape run so that a full session (initial scrape +
-- N retries) is represented as a single row. `details_failures` and
-- `open_status_failures` are also updated in-place to reflect the latest state.
ALTER TABLE scrape_runs
    ADD COLUMN retry_count   INT     NOT NULL DEFAULT 0,
    ADD COLUMN last_retry_at TIMESTAMPTZ;
