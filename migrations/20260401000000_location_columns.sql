ALTER TABLE coming_soon_superchargers
    ADD COLUMN city   TEXT,
    ADD COLUMN region TEXT;

CREATE INDEX ON coming_soon_superchargers (region);
