-- Assistant and tool writes reuse identities within one conversation and role.
CREATE INDEX idx_chat_non_user_save_id ON conversation_messages (
    conversation_id,
    role,
    CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END
) WHERE role <> 'user' AND metadata IS NOT NULL
    AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END IS NOT NULL;
