CREATE SCHEMA IF NOT EXISTS app;

CREATE TABLE IF NOT EXISTS app.user_identity (
    user_id TEXT PRIMARY KEY,
    sub TEXT NOT NULL,
    iss TEXT,
    email TEXT,
    name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS app.user_session (
    session_key TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES app.user_identity(user_id) ON DELETE CASCADE,
    jti TEXT,
    issuer TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS app.query_log (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES app.user_identity(user_id) ON DELETE CASCADE,
    session_key TEXT REFERENCES app.user_session(session_key) ON DELETE SET NULL,
    source TEXT NOT NULL,
    operation TEXT NOT NULL,
    request_payload JSONB NOT NULL,
    response_payload JSONB,
    succeeded BOOLEAN NOT NULL,
    duration_ms INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_query_log_user_created_at
ON app.query_log(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_query_log_operation_created_at
ON app.query_log(operation, created_at DESC);
