ALTER TABLE coming_soon_superchargers
    ADD COLUMN open_status_check_failed BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX ON coming_soon_superchargers (open_status_check_failed)
    WHERE open_status_check_failed = TRUE;

ALTER TABLE scrape_runs
    ADD COLUMN open_status_failures INT NOT NULL DEFAULT 0;
