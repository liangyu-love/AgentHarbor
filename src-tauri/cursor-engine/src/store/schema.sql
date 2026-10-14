-- Conversation state for Cursor runs served by AgentHarbor. Derived from
-- cursor-byok's migrated schema (MIT); model and trace tables are left out.
CREATE TABLE IF NOT EXISTS conversations (
    conversation_id TEXT PRIMARY KEY,
    current_checkpoint_id INTEGER,
    active_run_id TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    role TEXT NOT NULL,
    origin TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    runtime_event_id TEXT,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, message_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS messages_runtime_event
ON messages(conversation_id, runtime_event_id)
WHERE runtime_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS conversation_checkpoints (
    checkpoint_id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    parent_checkpoint_id INTEGER,
    state_digest BLOB NOT NULL CHECK(length(state_digest) = 32),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (conversation_id, state_digest),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id),
    FOREIGN KEY (parent_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id)
);

CREATE INDEX IF NOT EXISTS conversation_checkpoints_parent
ON conversation_checkpoints(conversation_id, parent_checkpoint_id);

CREATE TABLE IF NOT EXISTS checkpoint_messages (
    checkpoint_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    PRIMARY KEY (checkpoint_id, ordinal),
    UNIQUE (checkpoint_id, message_id),
    FOREIGN KEY (checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id),
    FOREIGN KEY (conversation_id, message_id) REFERENCES messages(conversation_id, message_id)
);

CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    base_checkpoint_id INTEGER NOT NULL,
    head_checkpoint_id INTEGER NOT NULL,
    parent_run_id TEXT,
    parent_tool_call_id TEXT,
    run_kind TEXT NOT NULL,
    subagent_kind TEXT,
    status TEXT NOT NULL,
    provider_call_index INTEGER NOT NULL DEFAULT -1,
    turn_usage_json TEXT NOT NULL DEFAULT 'null',
    failure_category TEXT,
    failure_summary TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    cursor_request_id TEXT,
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id),
    FOREIGN KEY (base_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id),
    FOREIGN KEY (head_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id)
);

CREATE INDEX IF NOT EXISTS runs_conversation_status ON runs(conversation_id, status);
CREATE INDEX IF NOT EXISTS idx_runs_cursor_request_active
ON runs(cursor_request_id, status, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS tool_rounds (
    round_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    base_checkpoint_id INTEGER NOT NULL,
    assistant_json TEXT NOT NULL,
    status TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 0,
    next_completion_seq INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (run_id) REFERENCES runs(run_id),
    FOREIGN KEY (base_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id)
);

CREATE INDEX IF NOT EXISTS tool_rounds_run_status ON tool_rounds(run_id, status);

CREATE TABLE IF NOT EXISTS tool_round_calls (
    round_id TEXT NOT NULL,
    call_index INTEGER NOT NULL,
    call_id TEXT NOT NULL,
    model_call_id TEXT NOT NULL,
    name TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    status TEXT NOT NULL,
    completion_seq INTEGER,
    result_content TEXT,
    result_is_error INTEGER,
    committed_checkpoint_id INTEGER,
    completed_at_ms INTEGER,
    argument_error TEXT,
    PRIMARY KEY (round_id, call_index),
    UNIQUE (round_id, call_id),
    UNIQUE (round_id, completion_seq),
    FOREIGN KEY (round_id) REFERENCES tool_rounds(round_id),
    FOREIGN KEY (committed_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id)
);

CREATE TABLE IF NOT EXISTS blobs (
    blob_id BLOB PRIMARY KEY CHECK(length(blob_id) = 32),
    data BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS blob_edges (
    parent_blob_id BLOB NOT NULL,
    child_blob_id BLOB NOT NULL,
    field_name TEXT NOT NULL,
    PRIMARY KEY (parent_blob_id, child_blob_id, field_name),
    FOREIGN KEY (parent_blob_id) REFERENCES blobs(blob_id),
    FOREIGN KEY (child_blob_id) REFERENCES blobs(blob_id)
);

CREATE INDEX IF NOT EXISTS blob_edges_child ON blob_edges(child_blob_id);

CREATE TABLE IF NOT EXISTS input_anchors (
    conversation_id TEXT NOT NULL,
    input_id TEXT NOT NULL,
    base_checkpoint_id INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, input_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id),
    FOREIGN KEY (base_checkpoint_id) REFERENCES conversation_checkpoints(checkpoint_id)
);

CREATE TABLE IF NOT EXISTS service_settings (
    setting_key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO service_settings(setting_key, value_json, updated_at_ms)
VALUES ('llm_detailed_logging', 'false', 0);

CREATE TABLE IF NOT EXISTS llm_calls (
    call_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    provider_call_index INTEGER NOT NULL,
    model_hash TEXT,
    provider_type TEXT NOT NULL,
    provider_url TEXT NOT NULL,
    request_type TEXT NOT NULL,
    request_url TEXT NOT NULL,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL,
    finish_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    request_started_at_ms INTEGER,
    response_headers_at_ms INTEGER,
    first_event_at_ms INTEGER,
    first_text_at_ms INTEGER,
    finished_at_ms INTEGER,
    queue_ms INTEGER,
    ttfb_ms INTEGER,
    ttft_ms INTEGER,
    duration_ms INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    reasoning_tokens INTEGER,
    usage_json TEXT,
    message_count INTEGER NOT NULL,
    tool_count INTEGER NOT NULL,
    request_bytes INTEGER,
    response_bytes INTEGER NOT NULL DEFAULT 0,
    stream_event_count INTEGER NOT NULL DEFAULT 0,
    http_status INTEGER,
    error_kind TEXT,
    error_message TEXT,
    detailed INTEGER NOT NULL,
    reasoning_effort TEXT,
    fast INTEGER NOT NULL DEFAULT 0 CHECK (fast IN (0, 1)),
    first_valid_response_at_ms INTEGER,
    ttfr_ms INTEGER
);

CREATE INDEX IF NOT EXISTS llm_calls_created ON llm_calls(created_at_ms DESC);
CREATE INDEX IF NOT EXISTS llm_calls_run ON llm_calls(run_id, provider_call_index);

CREATE TABLE IF NOT EXISTS llm_call_requests (
    call_id TEXT PRIMARY KEY,
    headers_json TEXT NOT NULL,
    body_json TEXT NOT NULL,
    byte_count INTEGER NOT NULL,
    FOREIGN KEY(call_id) REFERENCES llm_calls(call_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS llm_call_response_chunks (
    call_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    received_offset_ms INTEGER NOT NULL,
    data BLOB NOT NULL,
    byte_count INTEGER NOT NULL,
    PRIMARY KEY(call_id, seq),
    FOREIGN KEY(call_id) REFERENCES llm_calls(call_id) ON DELETE CASCADE
);
