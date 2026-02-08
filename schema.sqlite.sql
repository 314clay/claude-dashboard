-- Dashboard Native: SQLite schema
-- Converted from PostgreSQL schema.sql
--
-- Default database location: ~/.config/dashboard-native/dashboard.db
-- Override with DB_PATH environment variable.
--
-- This file is idempotent: safe to run multiple times.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ============================================================
-- SESSIONS TABLE
-- Tracks Claude Code session lifecycle (start, end, working directory).
-- ============================================================
CREATE TABLE IF NOT EXISTS sessions (
    session_id        TEXT PRIMARY KEY,
    cwd               TEXT NOT NULL,
    transcript_path   TEXT,
    permission_mode   TEXT,
    source            TEXT,              -- 'startup', 'resume', 'clear', 'compact'
    end_reason        TEXT,
    status            TEXT DEFAULT 'active',
    parent_session_id TEXT,              -- fork/resume parent
    fork_message_num  INTEGER,           -- message number where fork occurred
    start_time        TEXT NOT NULL DEFAULT (datetime('now')),
    end_time          TEXT,
    created_at        TEXT DEFAULT (datetime('now')),
    updated_at        TEXT DEFAULT (datetime('now')),

    FOREIGN KEY (parent_session_id)
        REFERENCES sessions (session_id)
        ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_cwd        ON sessions (cwd);
CREATE INDEX IF NOT EXISTS idx_sessions_start_time ON sessions (start_time);
CREATE INDEX IF NOT EXISTS idx_sessions_parent     ON sessions (parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_status     ON sessions (status);

-- ============================================================
-- MESSAGES TABLE
-- User and assistant messages within sessions.
-- ============================================================
CREATE TABLE IF NOT EXISTS messages (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id            TEXT    NOT NULL,
    role                  TEXT    NOT NULL,   -- 'user' or 'assistant'
    content               TEXT    NOT NULL,
    sequence_num          INTEGER NOT NULL,
    timestamp             TEXT    DEFAULT (datetime('now')),
    token_count           INTEGER,            -- output token count (legacy name)
    output_tokens         INTEGER,            -- output tokens (explicit)
    input_tokens          INTEGER,
    cache_read_tokens     INTEGER,
    cache_creation_tokens INTEGER,
    model                 TEXT,
    importance_score      REAL,               -- 0.0 - 1.0
    importance_reason     TEXT,
    importance_scored_at  TEXT,
    created_at            TEXT    DEFAULT (datetime('now')),

    FOREIGN KEY (session_id)
        REFERENCES sessions (session_id)
        ON DELETE CASCADE,

    UNIQUE (session_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_messages_session   ON messages (session_id);
CREATE INDEX IF NOT EXISTS idx_messages_role      ON messages (role);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages (timestamp);

-- SQLite does not support partial indexes with WHERE on CREATE INDEX IF NOT EXISTS
-- in all versions, but it does support them in 3.8.0+ (2013). Safe for modern SQLite.
CREATE INDEX IF NOT EXISTS idx_messages_importance_score
    ON messages (importance_score)
    WHERE importance_score IS NOT NULL;

-- ============================================================
-- TOOL USAGES TABLE
-- Records each tool invocation within an assistant message.
-- ============================================================
CREATE TABLE IF NOT EXISTS tool_usages (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id   INTEGER NOT NULL,
    tool_name    TEXT    NOT NULL,
    tool_input   TEXT,                -- JSON string (was JSONB in PostgreSQL)
    tool_output  TEXT,
    sequence_num INTEGER,
    timestamp    TEXT    DEFAULT (datetime('now')),
    success      INTEGER DEFAULT 1,   -- 0/1 boolean
    created_at   TEXT    DEFAULT (datetime('now')),

    FOREIGN KEY (message_id)
        REFERENCES messages (id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tool_usages_message ON tool_usages (message_id);
CREATE INDEX IF NOT EXISTS idx_tool_usages_name    ON tool_usages (tool_name);

-- ============================================================
-- SESSION SUMMARIES TABLE
-- LLM-generated summaries with topics and project detection.
-- ============================================================
CREATE TABLE IF NOT EXISTS session_summaries (
    session_id           TEXT PRIMARY KEY,
    summary              TEXT,
    user_requests        TEXT,
    completed_work       TEXT,
    topics               TEXT,          -- JSON string: ["topic1", "topic2", ...]
    detected_project     TEXT,
    model                TEXT,
    message_count_at_gen INTEGER,       -- for staleness detection
    generated_at         TEXT DEFAULT (datetime('now')),

    FOREIGN KEY (session_id)
        REFERENCES sessions (session_id)
        ON DELETE CASCADE
);

-- ============================================================
-- SEMANTIC FILTERS TABLE
-- User-defined natural-language filters for graph visualization.
-- ============================================================
CREATE TABLE IF NOT EXISTS semantic_filters (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    query_text TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (datetime('now')),
    is_active  INTEGER NOT NULL DEFAULT 1,   -- 0/1 boolean

    UNIQUE (name)
);

-- ============================================================
-- SEMANTIC FILTER RESULTS TABLE
-- Per-message match results for each semantic filter.
-- ============================================================
CREATE TABLE IF NOT EXISTS semantic_filter_results (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    filter_id   INTEGER NOT NULL,
    message_id  INTEGER NOT NULL,
    matches     INTEGER NOT NULL,     -- 0/1 boolean
    confidence  REAL,
    scored_at   TEXT    NOT NULL DEFAULT (datetime('now')),

    FOREIGN KEY (filter_id)
        REFERENCES semantic_filters (id)
        ON DELETE CASCADE,

    FOREIGN KEY (message_id)
        REFERENCES messages (id)
        ON DELETE CASCADE,

    UNIQUE (filter_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_sfr_filter_matches ON semantic_filter_results (filter_id, matches);
CREATE INDEX IF NOT EXISTS idx_sfr_message         ON semantic_filter_results (message_id);

-- ============================================================
-- MESSAGE EMBEDDINGS TABLE
-- Vector embeddings for semantic similarity search.
-- ============================================================
CREATE TABLE IF NOT EXISTS message_embeddings (
    message_id  INTEGER PRIMARY KEY,
    model       TEXT    NOT NULL,
    dimensions  INTEGER NOT NULL,
    embedding   BLOB    NOT NULL,
    created_at  TEXT    DEFAULT (datetime('now')),

    FOREIGN KEY (message_id)
        REFERENCES messages (id)
        ON DELETE CASCADE
);

-- ============================================================
-- DAILY USAGE TABLE
-- Per-day, per-model token usage from stats-cache.json.
-- ============================================================
CREATE TABLE IF NOT EXISTS daily_usage (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    date                  TEXT    NOT NULL,   -- ISO8601 date: YYYY-MM-DD
    model                 TEXT    NOT NULL,
    input_tokens          INTEGER DEFAULT 0,
    output_tokens         INTEGER DEFAULT 0,
    cache_read_tokens     INTEGER DEFAULT 0,
    cache_creation_tokens INTEGER DEFAULT 0,
    message_count         INTEGER DEFAULT 0,
    session_count         INTEGER DEFAULT 0,
    tool_call_count       INTEGER DEFAULT 0,
    synced_at             TEXT    DEFAULT (datetime('now')),

    UNIQUE (date, model)
);

CREATE INDEX IF NOT EXISTS idx_daily_usage_date  ON daily_usage (date);
CREATE INDEX IF NOT EXISTS idx_daily_usage_model ON daily_usage (model);

-- ============================================================
-- MODEL USAGE TABLE
-- Cumulative per-model token stats from stats-cache.json.
-- ============================================================
CREATE TABLE IF NOT EXISTS model_usage (
    model                 TEXT PRIMARY KEY,
    input_tokens          INTEGER DEFAULT 0,
    output_tokens         INTEGER DEFAULT 0,
    cache_read_tokens     INTEGER DEFAULT 0,
    cache_creation_tokens INTEGER DEFAULT 0,
    web_search_requests   INTEGER DEFAULT 0,
    synced_at             TEXT    DEFAULT (datetime('now'))
);

-- ============================================================
-- OVERALL STATS TABLE
-- High-level aggregate stats (singleton row with id=1).
-- ============================================================
CREATE TABLE IF NOT EXISTS overall_stats (
    id                       INTEGER PRIMARY KEY,  -- always 1
    total_sessions           INTEGER DEFAULT 0,
    total_messages           INTEGER DEFAULT 0,
    longest_session_messages INTEGER DEFAULT 0,
    hour_counts              TEXT,                  -- JSON string: array of 24 integers
    stats_cache_date         TEXT,                  -- ISO8601 date: YYYY-MM-DD
    synced_at                TEXT DEFAULT (datetime('now'))
);
