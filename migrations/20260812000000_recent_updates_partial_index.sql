-- Speeds up recent-updates feed query by matching its filter and sort shape.
CREATE INDEX IF NOT EXISTS status_changes_recent_updates_idx
ON status_changes (changed_at DESC, id DESC)
WHERE new_status != 'REMOVED' AND new_status != 'UNKNOWN';
