ALTER TABLE conversation_messages
    ADD COLUMN sequence_num INTEGER;

WITH ranked_messages AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY conversation_id
            ORDER BY created_at ASC, id ASC
        ) - 1 AS sequence_num
    FROM conversation_messages
)
UPDATE conversation_messages AS message
SET sequence_num = ranked.sequence_num
FROM ranked_messages AS ranked
WHERE message.id = ranked.id;

ALTER TABLE conversation_messages
    ALTER COLUMN sequence_num SET NOT NULL;

CREATE UNIQUE INDEX idx_conversation_messages_sequence
    ON conversation_messages(conversation_id, sequence_num);
