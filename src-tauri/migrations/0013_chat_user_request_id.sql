-- A draft can outlive its WebView and the in-memory chat journal.
CREATE INDEX idx_chat_user_request_id ON conversation_messages (
    CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.client_request_id') END
) WHERE role = 'user' AND metadata IS NOT NULL;
