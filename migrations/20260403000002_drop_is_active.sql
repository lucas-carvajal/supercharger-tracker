ALTER TABLE coming_soon_superchargers DROP COLUMN is_active;
DROP INDEX IF EXISTS coming_soon_superchargers_is_active_idx;
