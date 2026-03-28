ALTER TABLE scrape_runs ADD COLUMN details_failures INT NOT NULL DEFAULT 0;
ALTER TABLE scrape_runs ADD COLUMN run_type TEXT NOT NULL DEFAULT 'full';

ALTER TABLE coming_soon_superchargers
    ADD COLUMN details_fetch_failed BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX ON coming_soon_superchargers (details_fetch_failed) WHERE details_fetch_failed = TRUE;
