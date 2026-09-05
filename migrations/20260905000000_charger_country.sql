ALTER TABLE coming_soon_superchargers
    ADD COLUMN country TEXT;

ALTER TABLE opened_superchargers
    ADD COLUMN country TEXT;

CREATE INDEX ON coming_soon_superchargers (country);
