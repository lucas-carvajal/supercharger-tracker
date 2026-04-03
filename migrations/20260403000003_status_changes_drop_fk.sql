-- Drop the FK so status_changes rows survive when an opened charger is deleted
-- from coming_soon_superchargers. Rows become soft-references, still queryable by id.
ALTER TABLE status_changes DROP CONSTRAINT status_changes_supercharger_id_fkey;
