ALTER TABLE coming_soon_superchargers ALTER COLUMN status DROP DEFAULT;
ALTER TABLE coming_soon_superchargers
    ALTER COLUMN status TYPE TEXT USING (
        CASE status::text
            WHEN 'IN_DEVELOPMENT'     THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION'
            ELSE status::text
        END);
ALTER TABLE coming_soon_superchargers ALTER COLUMN status SET DEFAULT 'UNKNOWN';

ALTER TABLE status_changes
    ALTER COLUMN old_status TYPE TEXT USING (
        CASE old_status::text
            WHEN 'IN_DEVELOPMENT'     THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION'
            ELSE old_status::text
        END),
    ALTER COLUMN new_status TYPE TEXT USING (
        CASE new_status::text
            WHEN 'IN_DEVELOPMENT'     THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION'
            ELSE new_status::text
        END);

DROP TYPE site_status;