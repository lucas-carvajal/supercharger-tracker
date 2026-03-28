ALTER TABLE coming_soon_superchargers
    ADD COLUMN details_fetch_failed BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX ON coming_soon_superchargers (details_fetch_failed) WHERE details_fetch_failed = TRUE;
