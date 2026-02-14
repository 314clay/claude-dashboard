"""Rule-based filter scoring — pure logic, no LLM calls.

Supported query_text patterns:
  - "role:user"      → matches messages where role == 'user'
  - "role:assistant"  → matches messages where role == 'assistant'
  - "has_tools"      → matches messages that have at least one tool_usages row
  - "tool:<name>"    → matches messages that used a specific tool (e.g. "tool:Bash")
  - "long"           → matches messages with content > 500 chars
  - "short"          → matches messages with content < 100 chars
"""
from datetime import datetime, timezone
from .queries import get_connection


def _batch_query(cur, sql_template, ids, batch_size=900):
    """Run a query in batches for large IN clauses (SQLite limit ~999)."""
    results = []
    for i in range(0, len(ids), batch_size):
        batch = ids[i:i + batch_size]
        placeholders = ",".join("?" for _ in batch)
        cur.execute(sql_template.format(placeholders=placeholders), batch)
        results.extend(cur.fetchall())
    return results


def score_rule_filter(filter_id: int, query_text: str) -> dict:
    """Score all unscored messages for a rule-based filter.

    Evaluates the rule against every message that doesn't already have
    a result row for this filter, then inserts results into
    semantic_filter_results with confidence=1.0.

    Returns:
        dict: { filter_id, scored, matches }
    """
    conn = get_connection()
    cur = conn.cursor()

    query = query_text.strip()
    query_lower = query.lower()
    needs_content = query_lower in ("long", "short")

    # Find messages not yet scored for this filter
    if needs_content:
        cur.execute("""
            SELECT m.id, m.role, LENGTH(m.content) as content_len
            FROM messages m
            WHERE m.id NOT IN (
                SELECT r.message_id FROM semantic_filter_results r WHERE r.filter_id = ?
            )
        """, (filter_id,))
    else:
        cur.execute("""
            SELECT m.id, m.role
            FROM messages m
            WHERE m.id NOT IN (
                SELECT r.message_id FROM semantic_filter_results r WHERE r.filter_id = ?
            )
        """, (filter_id,))
    unscored = cur.fetchall()

    if not unscored:
        cur.close()
        conn.close()
        return {"filter_id": filter_id, "scored": 0, "matches": 0}

    now = datetime.now(timezone.utc).isoformat()
    message_ids = [row["id"] for row in unscored]

    # Pre-fetch tool data if needed
    tool_message_ids = set()
    if query_lower == "has_tools" or query_lower.startswith("tool:"):
        if query_lower.startswith("tool:"):
            # Specific tool name (preserve original case from query_text)
            tool_name = query[5:]  # everything after "tool:"
            for i in range(0, len(message_ids), 900):
                batch = message_ids[i:i + 900]
                placeholders = ",".join("?" for _ in batch)
                cur.execute(f"""
                    SELECT DISTINCT message_id FROM tool_usages
                    WHERE message_id IN ({placeholders}) AND tool_name = ?
                """, batch + [tool_name])
                tool_message_ids.update(row["message_id"] for row in cur.fetchall())
        else:
            # has_tools — any tool
            rows = _batch_query(cur,
                "SELECT DISTINCT message_id FROM tool_usages WHERE message_id IN ({placeholders})",
                message_ids)
            tool_message_ids = {row["message_id"] for row in rows}

    # Evaluate each message
    results = []
    matches = 0
    for row in unscored:
        msg_id = row["id"]
        role = row["role"]

        if query_lower == "role:user":
            matched = role == "user"
        elif query_lower == "role:assistant":
            matched = role == "assistant"
        elif query_lower == "has_tools" or query_lower.startswith("tool:"):
            matched = msg_id in tool_message_ids
        elif query_lower == "long":
            matched = (row["content_len"] or 0) > 500
        elif query_lower == "short":
            matched = (row["content_len"] or 0) < 100
        else:
            matched = False

        match_int = 1 if matched else 0
        if matched:
            matches += 1
        results.append((filter_id, msg_id, match_int, 1.0, now))

    # Batch insert
    cur.executemany("""
        INSERT OR IGNORE INTO semantic_filter_results
            (filter_id, message_id, matches, confidence, scored_at)
        VALUES (?, ?, ?, ?, ?)
    """, results)
    conn.commit()

    scored = len(results)
    cur.close()
    conn.close()

    return {"filter_id": filter_id, "scored": scored, "matches": matches}
