-- Add user-owned character instance metadata and template reconciliation state.

ALTER TABLE characters ADD COLUMN template_id TEXT;
ALTER TABLE characters ADD COLUMN template_version TEXT;
ALTER TABLE characters ADD COLUMN template_snapshot_json TEXT;
ALTER TABLE characters ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE characters ADD COLUMN avatar_path TEXT;
ALTER TABLE characters ADD COLUMN greeting TEXT NOT NULL DEFAULT '';
ALTER TABLE characters ADD COLUMN greeting_consumed_at INTEGER;
ALTER TABLE characters ADD COLUMN greeting_message_id INTEGER;
ALTER TABLE characters ADD COLUMN example_dialogue TEXT NOT NULL DEFAULT '';
ALTER TABLE characters ADD COLUMN runtime_profile_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE characters ADD COLUMN user_modified_at INTEGER;

-- Existing characters must not emit a first-run greeting after upgrading.
UPDATE characters
SET greeting_consumed_at = COALESCE(NULLIF(updated_at, 0), NULLIF(created_at, 0), unixepoch())
WHERE greeting_consumed_at IS NULL;
