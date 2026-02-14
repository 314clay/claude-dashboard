"""Session summarization using LLM.

Routes through the shared LLM module for provider-agnostic API access.
"""
import json
import os
from datetime import datetime, timezone
from functools import lru_cache
from .queries import get_connection, get_session_messages, get_session_messages_before, _normalize_path
from . import llm

# In-memory cache for partial summaries (session_id, timestamp) -> result
_partial_summary_cache: dict[tuple[str, str], dict] = {}

SUMMARY_PROMPT = """Analyze this Claude Code conversation and provide a structured summary.

CONVERSATION:
{conversation}

Respond in exactly this JSON format (no markdown, just raw JSON):
{{
    "summary": "One paragraph overview of what happened in this session",
    "user_requests": "Bullet list of what the user asked for (use \\n for newlines)",
    "completed_work": "Bullet list of what was actually accomplished (use \\n for newlines)",
    "topics": ["topic1", "topic2", "topic3"]
}}

TOPIC GUIDELINES:
- Extract 2-5 topic tags that categorize this conversation
- Topics should be lowercase, hyphenated if multi-word (e.g., "data-visualization", "api-integration")
- Use consistent naming: prefer "database" over "db", "visualization" over "viz"
- Common topics: "debugging", "refactoring", "testing", "documentation", "deployment",
  "api-integration", "database", "visualization", "configuration", "research",
  "obsidian", "streamlit", "docker", "git"
- Be specific when relevant (e.g., "postgres" instead of just "database")

Be concise. Focus on the key points. If the conversation is about coding, mention the specific files or features involved."""


def _generate(prompt: str) -> str | None:
    """Generate text via the configured LLM provider."""
    if not llm.is_available():
        print("LLM not configured - AI summaries disabled")
        return None
    return llm.complete(
        messages=[{"role": "user", "content": prompt}],
        max_tokens=2048,
    )


def _extract_json(text: str) -> dict | None:
    """Extract JSON object from LLM response, handling markdown and extra text."""
    import re

    text = text.strip()

    # Try to extract from markdown code block first
    if "```" in text:
        match = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", text, re.DOTALL)
        if match:
            try:
                return json.loads(match.group(1))
            except json.JSONDecodeError:
                pass

    # Try to find JSON object directly
    match = re.search(r"\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}", text, re.DOTALL)
    if match:
        try:
            return json.loads(match.group(0))
        except json.JSONDecodeError:
            pass

    # Try the whole text as JSON
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass

    return None


def generate_session_summary(session_id: str) -> dict | None:
    """Generate a summary for a session using the configured LLM."""
    messages = get_session_messages(session_id)
    if not messages:
        return None

    # Build conversation text
    conversation_parts = []
    for msg in messages:
        role = "USER" if msg['role'] == 'user' else ("CLAUDE" if msg['role'] == 'assistant' else f"AGENT({msg['role']})")
        content = msg['content'] or ""
        if len(content) > 2000:
            content = content[:2000] + "..."
        conversation_parts.append(f"{role}: {content}")

    conversation_text = "\n\n".join(conversation_parts)

    if len(conversation_text) > 50000:
        conversation_text = conversation_text[:50000] + "\n\n[CONVERSATION TRUNCATED]"

    prompt = SUMMARY_PROMPT.format(conversation=conversation_text)

    text = _generate(prompt)
    if text is None:
        return None

    try:
        result = _extract_json(text)
        if result is None:
            print(f"Could not extract JSON from response: {text[:200]}...")
            return None
        return {
            "summary": result.get("summary", ""),
            "user_requests": result.get("user_requests", ""),
            "completed_work": result.get("completed_work", ""),
            "topics": result.get("topics", []),
        }
    except Exception as e:
        print(f"Error parsing summary: {e}")
        return None


PARTIAL_SUMMARY_PROMPT = """Analyze this partial Claude Code conversation (up to a specific point in time) and provide a structured summary.

CONVERSATION:
{conversation}

Respond in exactly this JSON format (no markdown, just raw JSON):
{{
    "summary": "One paragraph overview of what happened up to this point",
    "completed_work": "Bullet list of what was successfully accomplished (use \\n for newlines)",
    "unsuccessful_attempts": "Bullet list of things that were tried but failed or remain unfinished (use \\n for newlines)",
    "current_focus": "What the conversation was actively working on at this point"
}}

Focus on:
1. What was the user trying to achieve?
2. What actually got done successfully?
3. What was attempted but didn't work out?
4. What was being worked on at this exact moment?

Be specific about failures - mention error messages, rejected approaches, or incomplete implementations.
Be concise. Focus on key points."""


def generate_partial_summary(session_id: str, before_timestamp: str) -> dict | None:
    """Generate a summary for a session up to a specific timestamp.

    Args:
        session_id: The session UUID
        before_timestamp: ISO8601 timestamp - only summarize messages at or before this time

    Returns:
        dict with summary, completed_work, unsuccessful_attempts, current_focus,
        user_count, and assistant_count
    """
    cache_key = (session_id, before_timestamp)
    if cache_key in _partial_summary_cache:
        print(f"Cache hit for partial summary: {session_id[:8]}...@{before_timestamp}")
        return _partial_summary_cache[cache_key]

    print(f"Cache miss - generating partial summary for {session_id[:8]}...@{before_timestamp}")

    messages = get_session_messages_before(session_id, before_timestamp)
    if not messages:
        return None

    user_count = sum(1 for m in messages if m['role'] == 'user')
    assistant_count = sum(1 for m in messages if m['role'] == 'assistant')

    conversation_parts = []
    for msg in messages:
        role = "USER" if msg['role'] == 'user' else ("CLAUDE" if msg['role'] == 'assistant' else f"AGENT({msg['role']})")
        content = msg['content'] or ""
        if len(content) > 2000:
            content = content[:2000] + "..."
        conversation_parts.append(f"{role}: {content}")

    conversation_text = "\n\n".join(conversation_parts)

    if len(conversation_text) > 50000:
        conversation_text = conversation_text[:50000] + "\n\n[CONVERSATION TRUNCATED]"

    prompt = PARTIAL_SUMMARY_PROMPT.format(conversation=conversation_text)

    text = _generate(prompt)
    if text is None:
        return {
            "summary": "Failed to generate summary",
            "completed_work": "",
            "unsuccessful_attempts": "",
            "current_focus": "",
            "user_count": user_count,
            "assistant_count": assistant_count,
        }

    try:
        result = _extract_json(text)
        if result is None:
            print(f"Could not extract JSON from partial summary: {text[:200]}...")
            return {
                "summary": "Failed to parse summary response",
                "completed_work": "",
                "unsuccessful_attempts": "",
                "current_focus": "",
                "user_count": user_count,
                "assistant_count": assistant_count,
            }

        def ensure_string(val):
            if isinstance(val, list):
                return "\n".join(str(v) for v in val) if val else ""
            return str(val) if val else ""

        summary_result = {
            "summary": ensure_string(result.get("summary", "")),
            "completed_work": ensure_string(result.get("completed_work", "")),
            "unsuccessful_attempts": ensure_string(result.get("unsuccessful_attempts", "")),
            "current_focus": ensure_string(result.get("current_focus", "")),
            "user_count": user_count,
            "assistant_count": assistant_count,
        }
        _partial_summary_cache[cache_key] = summary_result
        return summary_result
    except Exception as e:
        print(f"Error parsing partial summary: {e}")
        return {
            "summary": f"Error parsing response: {e}",
            "completed_work": "",
            "unsuccessful_attempts": "",
            "current_focus": "",
            "user_count": user_count,
            "assistant_count": assistant_count,
        }


def get_or_create_summary(session_id: str, force_refresh: bool = False) -> dict | None:
    """Get existing summary or create a new one."""
    conn = get_connection()
    cur = conn.cursor()

    # Check for existing summary
    if not force_refresh:
        cur.execute("""
            SELECT summary, user_requests, completed_work, topics, generated_at, model
            FROM session_summaries
            WHERE session_id = ?
        """, (session_id,))
        row = cur.fetchone()
        if row:
            cur.close()
            conn.close()
            result = dict(row)
            # Parse JSON topics
            if isinstance(result.get('topics'), str):
                try:
                    result['topics'] = json.loads(result['topics'])
                except (json.JSONDecodeError, TypeError):
                    result['topics'] = []
            return result

    # Generate new summary
    summary_data = generate_session_summary(session_id)
    if not summary_data:
        cur.close()
        conn.close()
        return None

    # Save to database
    model_name = llm.get_provider() or "unknown"

    cur.execute("""
        INSERT INTO session_summaries
            (session_id, summary, user_requests, completed_work, topics, model, generated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (session_id) DO UPDATE SET
            summary = excluded.summary,
            user_requests = excluded.user_requests,
            completed_work = excluded.completed_work,
            topics = excluded.topics,
            generated_at = excluded.generated_at,
            model = excluded.model
    """, (
        session_id,
        summary_data["summary"],
        summary_data["user_requests"],
        summary_data["completed_work"],
        json.dumps(summary_data.get("topics", [])),
        model_name,
        datetime.now(timezone.utc).isoformat(),
    ))

    conn.commit()

    # Fetch back the inserted row
    cur.execute("""
        SELECT summary, user_requests, completed_work, topics, generated_at, model
        FROM session_summaries
        WHERE session_id = ?
    """, (session_id,))

    result_row = cur.fetchone()
    cur.close()
    conn.close()

    if result_row:
        result = dict(result_row)
        if isinstance(result.get('topics'), str):
            try:
                result['topics'] = json.loads(result['topics'])
            except (json.JSONDecodeError, TypeError):
                result['topics'] = []
        return result
    return summary_data


def get_sessions_with_summaries(hours: float = 24, limit: int = 50) -> list[dict]:
    """Get sessions with their summaries for the card view."""
    conn = get_connection()
    cur = conn.cursor()

    from datetime import timedelta
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    cur.execute("""
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            s.end_time,
            COUNT(m.id) as total_messages,
            SUM(CASE WHEN m.role = 'user' THEN 1 ELSE 0 END) as user_messages,
            SUM(CASE WHEN m.role = 'assistant' THEN 1 ELSE 0 END) as assistant_messages,
            (strftime('%s', COALESCE(s.end_time, datetime('now'))) - strftime('%s', s.start_time)) / 60.0 as duration_mins,
            ss.summary,
            ss.user_requests,
            ss.completed_work,
            ss.topics
        FROM sessions s
        LEFT JOIN messages m ON s.session_id = m.session_id
        LEFT JOIN session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= ?
        GROUP BY s.session_id, s.cwd, s.start_time, s.end_time,
                 ss.summary, ss.user_requests, ss.completed_work, ss.topics
        ORDER BY s.start_time DESC
        LIMIT ?
    """, (since, limit))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    sessions = []
    for row in rows:
        session = dict(row)
        session['project'] = _normalize_path(session['cwd']) if session['cwd'] else ''
        session['is_active'] = session['end_time'] is None
        # Parse JSON topics
        if isinstance(session.get('topics'), str):
            try:
                session['topics'] = json.loads(session['topics'])
            except (json.JSONDecodeError, TypeError):
                session['topics'] = []
        sessions.append(session)

    return sessions


# In-memory cache for neighborhood summaries: frozenset(message_ids) -> result
_neighborhood_summary_cache: dict[frozenset, dict] = {}

NEIGHBORHOOD_SUMMARY_PROMPT = """Analyze this cluster of graph-adjacent Claude Code messages and provide a structured summary.
These messages are from nodes that are directly connected in a conversation graph.

MESSAGES (grouped by session):
{conversation}

Respond in exactly this JSON format (no markdown, just raw JSON):
{{
    "summary": "One paragraph overview of what this cluster of messages is about and how they relate",
    "themes": "Comma-separated list of recurring themes across these messages",
    "node_count": {node_count},
    "session_count": {session_count}
}}

Focus on:
1. What connects these messages? Are they about the same feature, bug, or topic?
2. What themes emerge across the different sessions?
3. What was being accomplished across this cluster?

Be concise. Focus on the big picture of what this neighborhood represents."""


def generate_neighborhood_summary(message_ids: list[int]) -> dict | None:
    """Generate a summary for a neighborhood of graph-adjacent messages.

    Args:
        message_ids: List of message IDs (the clicked node + its neighbors)

    Returns:
        dict with summary, themes, node_count, session_count
    """
    from .queries import get_messages_by_ids

    cache_key = frozenset(message_ids)
    if cache_key in _neighborhood_summary_cache:
        print(f"Cache hit for neighborhood summary: {len(message_ids)} nodes")
        return _neighborhood_summary_cache[cache_key]

    print(f"Generating neighborhood summary for {len(message_ids)} nodes")

    messages = get_messages_by_ids(message_ids)
    if not messages:
        return None

    # Group by session
    sessions: dict[str, list] = {}
    for msg in messages:
        sid = msg.get('session_id', 'unknown')
        sessions.setdefault(sid, []).append(msg)

    # Build labeled conversation text
    conversation_parts = []
    for sid, msgs in sessions.items():
        cwd = msgs[0].get('cwd', '') if msgs else ''
        conversation_parts.append(f"--- Session {sid[:8]} ({cwd}) ---")
        for msg in msgs:
            role = "USER" if msg['role'] == 'user' else ("CLAUDE" if msg['role'] == 'assistant' else f"AGENT({msg['role']})")
            content = msg.get('content', '') or ""
            if len(content) > 2000:
                content = content[:2000] + "..."
            conversation_parts.append(f"{role}: {content}")

    conversation_text = "\n\n".join(conversation_parts)
    if len(conversation_text) > 50000:
        conversation_text = conversation_text[:50000] + "\n\n[TRUNCATED]"

    prompt = NEIGHBORHOOD_SUMMARY_PROMPT.format(
        conversation=conversation_text,
        node_count=len(messages),
        session_count=len(sessions),
    )

    text = _generate(prompt)
    if text is None:
        return {
            "summary": "Failed to generate summary (LLM not configured)",
            "themes": "",
            "node_count": len(messages),
            "session_count": len(sessions),
        }

    try:
        result = _extract_json(text)
        if result is None:
            print(f"Could not extract JSON from neighborhood summary: {text[:200]}...")
            return {
                "summary": "Failed to parse summary response",
                "themes": "",
                "node_count": len(messages),
                "session_count": len(sessions),
            }

        def ensure_string(val):
            if isinstance(val, list):
                return ", ".join(str(v) for v in val) if val else ""
            return str(val) if val else ""

        summary_result = {
            "summary": ensure_string(result.get("summary", "")),
            "themes": ensure_string(result.get("themes", "")),
            "node_count": len(messages),
            "session_count": len(sessions),
        }
        _neighborhood_summary_cache[cache_key] = summary_result
        return summary_result
    except Exception as e:
        print(f"Error parsing neighborhood summary: {e}")
        return {
            "summary": f"Error: {e}",
            "themes": "",
            "node_count": len(messages),
            "session_count": len(sessions),
        }
