-- Dashboard Native: Standalone schema for claude_sessions
-- Derived from ConnectingServices migrations 003, 006, 007, 012, 018,
-- plus manual column additions (input_tokens, cache_read/creation_tokens)
-- and the session_summaries table (created outside migrations).
--
-- This file is idempotent: safe to run multiple times.

-- ============================================================
-- SCHEMA
-- ============================================================
CREATE SCHEMA IF NOT EXISTS claude_sessions;

-- ============================================================
-- SESSIONS TABLE
-- Tracks Claude Code session lifecycle (start, end, working directory).
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.sessions (
    session_id       VARCHAR(255) PRIMARY KEY,
    cwd              TEXT         NOT NULL,
    transcript_path  TEXT,
    permission_mode  VARCHAR(50),
    source           VARCHAR(50),          -- 'startup', 'resume', 'clear', 'compact'
    end_reason       VARCHAR(50),
    status           VARCHAR(20)  DEFAULT 'active',
    parent_session_id VARCHAR(255),        -- fork/resume parent
    fork_message_num INTEGER,              -- message number where fork occurred
    start_time       TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    end_time         TIMESTAMPTZ,
    created_at       TIMESTAMPTZ  DEFAULT NOW(),
    updated_at       TIMESTAMPTZ  DEFAULT NOW(),

    CONSTRAINT fk_sessions_parent
        FOREIGN KEY (parent_session_id)
        REFERENCES claude_sessions.sessions (session_id)
        ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_cwd        ON claude_sessions.sessions (cwd);
CREATE INDEX IF NOT EXISTS idx_sessions_start_time ON claude_sessions.sessions (start_time);
CREATE INDEX IF NOT EXISTS idx_sessions_parent     ON claude_sessions.sessions (parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_status     ON claude_sessions.sessions (status);

-- ============================================================
-- MESSAGES TABLE
-- User and assistant messages within sessions.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.messages (
    id                    SERIAL       PRIMARY KEY,
    session_id            VARCHAR(255) NOT NULL,
    role                  VARCHAR(20)  NOT NULL,   -- 'user' or 'assistant'
    content               TEXT         NOT NULL,
    sequence_num          INTEGER      NOT NULL,
    timestamp             TIMESTAMPTZ  DEFAULT NOW(),
    token_count           INTEGER,                 -- output token count (legacy name)
    output_tokens         INTEGER,                 -- output tokens (explicit)
    input_tokens          INTEGER,
    cache_read_tokens     INTEGER,
    cache_creation_tokens INTEGER,
    model                 VARCHAR(100),
    importance_score      FLOAT,                   -- 0.0 - 1.0
    importance_reason     VARCHAR(255),
    importance_scored_at  TIMESTAMPTZ,
    created_at            TIMESTAMPTZ  DEFAULT NOW(),

    CONSTRAINT fk_messages_session
        FOREIGN KEY (session_id)
        REFERENCES claude_sessions.sessions (session_id)
        ON DELETE CASCADE,

    CONSTRAINT uq_messages_session_sequence
        UNIQUE (session_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_messages_session   ON claude_sessions.messages (session_id);
CREATE INDEX IF NOT EXISTS idx_messages_role      ON claude_sessions.messages (role);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON claude_sessions.messages (timestamp);

-- Partial index: only index rows that have been scored
CREATE INDEX IF NOT EXISTS idx_messages_importance_score
    ON claude_sessions.messages (importance_score)
    WHERE importance_score IS NOT NULL;

-- ============================================================
-- TOOL USAGES TABLE
-- Records each tool invocation within an assistant message.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.tool_usages (
    id           SERIAL       PRIMARY KEY,
    message_id   INTEGER      NOT NULL,
    tool_name    VARCHAR(255) NOT NULL,
    tool_input   JSONB,
    tool_output  TEXT,
    sequence_num INTEGER,
    timestamp    TIMESTAMPTZ  DEFAULT NOW(),
    success      BOOLEAN      DEFAULT TRUE,
    created_at   TIMESTAMPTZ  DEFAULT NOW(),

    CONSTRAINT fk_tool_usages_message
        FOREIGN KEY (message_id)
        REFERENCES claude_sessions.messages (id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tool_usages_message ON claude_sessions.tool_usages (message_id);
CREATE INDEX IF NOT EXISTS idx_tool_usages_name    ON claude_sessions.tool_usages (tool_name);

-- ============================================================
-- SESSION SUMMARIES TABLE
-- LLM-generated summaries with topics and project detection.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.session_summaries (
    session_id          VARCHAR(255) PRIMARY KEY,
    summary             TEXT,
    user_requests       TEXT,
    completed_work      TEXT,
    topics              JSONB,                    -- ["topic1", "topic2", ...]
    detected_project    TEXT,
    model               VARCHAR(100),
    message_count_at_gen INTEGER,                 -- for staleness detection
    generated_at        TIMESTAMPTZ  DEFAULT NOW(),

    CONSTRAINT fk_session_summaries_session
        FOREIGN KEY (session_id)
        REFERENCES claude_sessions.sessions (session_id)
        ON DELETE CASCADE
);

-- ============================================================
-- SEMANTIC FILTERS TABLE
-- User-defined natural-language filters for graph visualization.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.semantic_filters (
    id          SERIAL       PRIMARY KEY,
    name        VARCHAR(100) NOT NULL,
    query_text  TEXT         NOT NULL,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    is_active   BOOLEAN      NOT NULL DEFAULT TRUE,

    CONSTRAINT uq_semantic_filters_name UNIQUE (name)
);

-- ============================================================
-- SEMANTIC FILTER RESULTS TABLE
-- Per-message match results for each semantic filter.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.semantic_filter_results (
    id          SERIAL       PRIMARY KEY,
    filter_id   INTEGER      NOT NULL,
    message_id  BIGINT       NOT NULL,
    matches     BOOLEAN      NOT NULL,
    confidence  FLOAT,
    scored_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT fk_sfr_filter
        FOREIGN KEY (filter_id)
        REFERENCES claude_sessions.semantic_filters (id)
        ON DELETE CASCADE,

    CONSTRAINT fk_sfr_message
        FOREIGN KEY (message_id)
        REFERENCES claude_sessions.messages (id)
        ON DELETE CASCADE,

    CONSTRAINT uq_sfr_filter_message UNIQUE (filter_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_sfr_filter_matches ON claude_sessions.semantic_filter_results (filter_id, matches);
CREATE INDEX IF NOT EXISTS idx_sfr_message         ON claude_sessions.semantic_filter_results (message_id);

-- ============================================================
-- DAILY USAGE TABLE
-- Per-day, per-model token usage from stats-cache.json.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.daily_usage (
    id                    SERIAL       PRIMARY KEY,
    date                  DATE         NOT NULL,
    model                 VARCHAR(100) NOT NULL,
    input_tokens          BIGINT       DEFAULT 0,
    output_tokens         BIGINT       DEFAULT 0,
    cache_read_tokens     BIGINT       DEFAULT 0,
    cache_creation_tokens BIGINT       DEFAULT 0,
    message_count         INTEGER      DEFAULT 0,
    session_count         INTEGER      DEFAULT 0,
    tool_call_count       INTEGER      DEFAULT 0,
    synced_at             TIMESTAMPTZ  DEFAULT NOW(),

    CONSTRAINT uq_daily_usage_date_model UNIQUE (date, model)
);

CREATE INDEX IF NOT EXISTS idx_daily_usage_date  ON claude_sessions.daily_usage (date);
CREATE INDEX IF NOT EXISTS idx_daily_usage_model ON claude_sessions.daily_usage (model);

-- ============================================================
-- MODEL USAGE TABLE
-- Cumulative per-model token stats from stats-cache.json.
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.model_usage (
    model                 VARCHAR(100) PRIMARY KEY,
    input_tokens          BIGINT       DEFAULT 0,
    output_tokens         BIGINT       DEFAULT 0,
    cache_read_tokens     BIGINT       DEFAULT 0,
    cache_creation_tokens BIGINT       DEFAULT 0,
    web_search_requests   INTEGER      DEFAULT 0,
    synced_at             TIMESTAMPTZ  DEFAULT NOW()
);

-- ============================================================
-- OVERALL STATS TABLE
-- High-level aggregate stats (singleton row with id=1).
-- ============================================================
CREATE TABLE IF NOT EXISTS claude_sessions.overall_stats (
    id                       INTEGER PRIMARY KEY,  -- always 1
    total_sessions           INTEGER DEFAULT 0,
    total_messages           INTEGER DEFAULT 0,
    longest_session_messages INTEGER DEFAULT 0,
    hour_counts              JSONB,                -- array of 24 integers
    stats_cache_date         DATE,
    synced_at                TIMESTAMPTZ DEFAULT NOW()
);
