-- Persist the memory revision captured when a dreaming proposal is created.
-- A proposal must never silently apply to a deleted, edited, or cross-character
-- memory after it has been reviewed.
ALTER TABLE memory_dream_proposals
    ADD COLUMN source_memory_versions TEXT NOT NULL DEFAULT '[]';
