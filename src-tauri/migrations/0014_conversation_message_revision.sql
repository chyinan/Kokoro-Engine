-- Guard history rewrites against changes to their own conversation.
ALTER TABLE conversations ADD COLUMN message_revision INTEGER NOT NULL DEFAULT 0;

CREATE TRIGGER conversation_message_revision_insert AFTER INSERT ON conversation_messages BEGIN
    UPDATE conversations SET message_revision = message_revision + 1 WHERE id = new.conversation_id;
END;

CREATE TRIGGER conversation_message_revision_update AFTER UPDATE ON conversation_messages BEGIN
    UPDATE conversations SET message_revision = message_revision + 1
    WHERE id IN (old.conversation_id, new.conversation_id);
END;

CREATE TRIGGER conversation_message_revision_delete AFTER DELETE ON conversation_messages BEGIN
    UPDATE conversations SET message_revision = message_revision + 1 WHERE id = old.conversation_id;
END;
