-- recent-* feeds now load status_changes and filter in domain code.
-- The old partial index only matched the SQL-side recent-changes predicate.
DROP INDEX IF EXISTS status_changes_recent_feed_idx;
