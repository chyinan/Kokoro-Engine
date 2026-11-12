-- A cheap invalidation stamp for readers. Update it in the same transaction
-- as the messages, including edits/imports that do not emit chat turn events.
CREATE TABLE chat_history_revision (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
INSERT INTO chat_history_revision (id, revision) VALUES (1, 0);

CREATE TRIGGER chat_history_revision_insert AFTER INSERT ON conversation_messages BEGIN
    UPDATE chat_history_revision SET revision = revision + 1 WHERE id = 1;
END;
CREATE TRIGGER chat_history_revision_update AFTER UPDATE ON conversation_messages BEGIN
    UPDATE chat_history_revision SET revision = revision + 1 WHERE id = 1;
END;
CREATE TRIGGER chat_history_revision_delete AFTER DELETE ON conversation_messages BEGIN
    UPDATE chat_history_revision SET revision = revision + 1 WHERE id = 1;
END;
