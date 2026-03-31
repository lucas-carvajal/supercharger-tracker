-- Migrate from Tesla UUID as primary key to location_url_slug as primary key.
-- Tesla's uuid field changes arbitrarily for the same physical location, making
-- it unreliable as a system identifier. The slug is stable and uniquely identifies
-- a location in both the Tesla API and our system.

-- Step 1: Remove entries that cannot be migrated (no slug means no stable identity).
DELETE FROM status_changes
WHERE supercharger_uuid IN (
    SELECT uuid FROM coming_soon_superchargers WHERE location_url_slug IS NULL
);
DELETE FROM coming_soon_superchargers WHERE location_url_slug IS NULL;

-- Step 2: Drop the existing FK so we can restructure.
ALTER TABLE status_changes DROP CONSTRAINT status_changes_supercharger_uuid_fkey;

-- Step 3: Migrate status_changes to reference slug instead of uuid.
ALTER TABLE status_changes ADD COLUMN slug TEXT;
UPDATE status_changes sc
SET slug = (
    SELECT location_url_slug
    FROM coming_soon_superchargers
    WHERE uuid = sc.supercharger_uuid
);
ALTER TABLE status_changes ALTER COLUMN slug SET NOT NULL;
ALTER TABLE status_changes DROP COLUMN supercharger_uuid;

-- Step 4: Drop the uuid PK and promote location_url_slug to primary key.
ALTER TABLE coming_soon_superchargers DROP CONSTRAINT coming_soon_superchargers_pkey;
ALTER TABLE coming_soon_superchargers DROP COLUMN uuid;
ALTER TABLE coming_soon_superchargers ALTER COLUMN location_url_slug SET NOT NULL;
ALTER TABLE coming_soon_superchargers RENAME COLUMN location_url_slug TO slug;
ALTER TABLE coming_soon_superchargers ADD PRIMARY KEY (slug);

-- Step 5: Re-add the FK from status_changes to coming_soon_superchargers.
ALTER TABLE status_changes
    ADD CONSTRAINT status_changes_slug_fkey
    FOREIGN KEY (slug) REFERENCES coming_soon_superchargers(slug);

-- Step 6: Recreate the index on the FK column (old one referenced the old column name).
DROP INDEX IF EXISTS status_changes_supercharger_uuid_idx;
CREATE INDEX ON status_changes (slug);
