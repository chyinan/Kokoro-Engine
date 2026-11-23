-- External ownership checks search by the saved user identity across conversations.
CREATE INDEX idx_chat_user_save_id ON conversation_messages (
    CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END
) WHERE role = 'user' AND metadata IS NOT NULL
    AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END IS NOT NULL;
