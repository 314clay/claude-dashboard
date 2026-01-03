---
name: db
description: Query the claude_sessions database for the dashboard. Use when working with sessions, messages, tool usage data, embeddings, or summaries.
allowed-tools: mcp__postgres-mcp-pro__execute_sql, mcp__postgres-mcp-pro__list_objects, mcp__postgres-mcp-pro__get_object_details, mcp__postgres-mcp-pro__explain_query
---

# Dashboard Database

PostgreSQL on port 5433, database `connectingservices`, schema `claude_sessions`.

## Tables

| Table | Purpose |
|-------|---------|
| `sessions` | session_id, cwd, start_time, end_time |
| `messages` | id, session_id, role, content, timestamp, sequence_num |
| `tool_usages` | tool_name, tool_input, timestamp, message_id |
| `session_summaries` | summary, topics, detected_project |
| `session_embeddings` | embedding (pgvector) |

## Common Queries

```sql
-- Recent sessions with message counts
SELECT s.session_id, s.cwd, s.start_time,
       COUNT(m.id) as msg_count
FROM claude_sessions.sessions s
LEFT JOIN claude_sessions.messages m ON s.session_id = m.session_id
GROUP BY s.session_id, s.cwd, s.start_time
ORDER BY s.start_time DESC LIMIT 10;

-- Messages for a session (for graph nodes)
SELECT id, role, LEFT(content, 100), timestamp, sequence_num
FROM claude_sessions.messages
WHERE session_id = 'xxx'
ORDER BY sequence_num;

-- Tool usage patterns
SELECT tool_name, COUNT(*) as uses
FROM claude_sessions.tool_usages
GROUP BY tool_name ORDER BY uses DESC;

-- Sessions with summaries
SELECT s.session_id, ss.summary, ss.topics
FROM claude_sessions.sessions s
JOIN claude_sessions.session_summaries ss ON s.session_id = ss.session_id
ORDER BY s.start_time DESC;
```

## API Integration

The Python API (`api/main.py`) wraps these queries. Add new endpoints there, then call from `src/api/client.rs`.
