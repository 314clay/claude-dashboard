---
name: db
description: Query the dashboard SQLite database. Use when working with sessions, messages, tool usage data, or summaries.
allowed-tools: Bash
---

# Dashboard Database

SQLite at `~/.config/dashboard-native/dashboard.db`.

## Tables

| Table | Purpose |
|-------|---------|
| `sessions` | session_id, cwd, start_time, end_time, source, parent_session_id |
| `messages` | id, session_id, role, content, timestamp, sequence_num, importance_score |
| `tool_usages` | tool_name, tool_input, timestamp, message_id |
| `session_summaries` | summary, topics, detected_project |
| `semantic_filters` | id, name, query_text, is_active |
| `semantic_filter_results` | filter_id, message_id, matches, confidence |
| `daily_usage` | date, model, token counts |
| `model_usage` | per-model cumulative stats |
| `overall_stats` | aggregate counts (singleton row) |

## Common Queries

```sql
-- Recent sessions with message counts
SELECT s.session_id, s.cwd, s.start_time,
       COUNT(m.id) as msg_count
FROM sessions s
LEFT JOIN messages m ON s.session_id = m.session_id
GROUP BY s.session_id, s.cwd, s.start_time
ORDER BY s.start_time DESC LIMIT 10;

-- Messages for a session (for graph nodes)
SELECT id, role, substr(content, 1, 100), timestamp, sequence_num
FROM messages
WHERE session_id = 'xxx'
ORDER BY sequence_num;

-- Tool usage patterns
SELECT tool_name, COUNT(*) as uses
FROM tool_usages
GROUP BY tool_name ORDER BY uses DESC;

-- Sessions with summaries
SELECT s.session_id, ss.summary, ss.topics
FROM sessions s
JOIN session_summaries ss ON s.session_id = ss.session_id
ORDER BY s.start_time DESC;
```

## Querying via CLI

```bash
sqlite3 ~/.config/dashboard-native/dashboard.db "SELECT COUNT(*) FROM sessions;"
```

## API Integration

The Python API (`api/main.py`) wraps these queries. Add new endpoints there, then call from `src/api/client.rs`.
