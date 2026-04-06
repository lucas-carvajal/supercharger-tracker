-- Add OPENED to site_status so graduations are recorded in status_changes like any
-- other transition. This lets export-diff pick up graduations from the same query
-- used for everything else, and removes the LEFT JOIN workaround in list_recent_changes.
ALTER TYPE site_status ADD VALUE IF NOT EXISTS 'OPENED';
