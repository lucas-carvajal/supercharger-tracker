-- Speeds up recent-changes feed query by matching its filter and sort shape.
CREATE INDEX IF NOT EXISTS status_changes_recent_feed_idx
ON status_changes (changed_at DESC, id DESC)
WHERE old_status IS NOT NULL AND new_status != 'UNKNOWN';
