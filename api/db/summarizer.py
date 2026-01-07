"""Session summarization using LiteLLM proxy.

Routes through LiteLLM proxy at port 4001 for unified API access.
"""
import os
from functools import lru_cache
from .queries import get_connection, get_session_messages, get_session_messages_before, _normalize_path

# In-memory cache for partial summaries (session_id, timestamp) -> result
_partial_summary_cache: dict[tuple[str, str], dict] = {}

# LiteLLM proxy configuration
LITELLM_BASE_URL = os.environ.get("LITELLM_BASE_URL", "http://localhost:4001")
LITELLM_API_KEY = os.environ.get("LITELLM_API_KEY", "sk-1234")  # Placeholder key
MODEL_NAME = os.environ.get("SUMMARY_MODEL", "gemini-2.5-flash")

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


def _generate_via_litellm(prompt: str) -> str | None:
    """Generate text via LiteLLM proxy."""
    try:
        from openai import OpenAI
        client = OpenAI(
            base_url=f"{LITELLM_BASE_URL}/v1",
            api_key=LITELLM_API_KEY
        )
        response = client.chat.completions.create(
            model=MODEL_NAME,
            messages=[{"role": "user", "content": prompt}],
            max_tokens=2048,
        )
        return response.choices[0].message.content
    except Exception as e:
        print(f"LiteLLM error: {e}")
        return None


def generate_session_summary(session_id: str) -> dict | None:
    """Generate a summary for a session using LiteLLM proxy."""
    # Get messages
    messages_df = get_session_messages(session_id)
    if messages_df.empty:
        return None

    # Build conversation text
    conversation_parts = []
    for _, msg in messages_df.iterrows():
        role = "USER" if msg['role'] == 'user' else "CLAUDE"
        content = msg['content'] or ""
        # Truncate very long messages
        if len(content) > 2000:
            content = content[:2000] + "..."
        conversation_parts.append(f"{role}: {content}")

    conversation_text = "\n\n".join(conversation_parts)

    # Limit total size to avoid token limits
    if len(conversation_text) > 50000:
        conversation_text = conversation_text[:50000] + "\n\n[CONVERSATION TRUNCATED]"

    prompt = SUMMARY_PROMPT.format(conversation=conversation_text)

    # Generate via LiteLLM
    text = _generate_via_litellm(prompt)
    if text is None:
        return None

    try:
        import json
        # Remove any markdown code blocks if present
        text = text.strip()
        if text.startswith("```"):
            text = text.split("```")[1]
            if text.startswith("json"):
                text = text[4:]
        text = text.strip()

        result = json.loads(text)
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
    # Check cache first
    cache_key = (session_id, before_timestamp)
    if cache_key in _partial_summary_cache:
        print(f"Cache hit for partial summary: {session_id[:8]}...@{before_timestamp}")
        return _partial_summary_cache[cache_key]

    print(f"Cache miss - generating partial summary for {session_id[:8]}...@{before_timestamp}")

    # Get messages up to timestamp
    messages_df = get_session_messages_before(session_id, before_timestamp)
    if messages_df.empty:
        return None

    # Count messages by role
    user_count = len(messages_df[messages_df['role'] == 'user'])
    assistant_count = len(messages_df[messages_df['role'] == 'assistant'])

    # Build conversation text
    conversation_parts = []
    for _, msg in messages_df.iterrows():
        role = "USER" if msg['role'] == 'user' else "CLAUDE"
        content = msg['content'] or ""
        if len(content) > 2000:
            content = content[:2000] + "..."
        conversation_parts.append(f"{role}: {content}")

    conversation_text = "\n\n".join(conversation_parts)

    # Limit total size
    if len(conversation_text) > 50000:
        conversation_text = conversation_text[:50000] + "\n\n[CONVERSATION TRUNCATED]"

    prompt = PARTIAL_SUMMARY_PROMPT.format(conversation=conversation_text)

    # Generate via LiteLLM
    text = _generate_via_litellm(prompt)
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
        import json
        text = text.strip()
        if text.startswith("```"):
            text = text.split("```")[1]
            if text.startswith("json"):
                text = text[4:]
        text = text.strip()

        result = json.loads(text)

        # Ensure string fields are always strings (Gemini sometimes returns arrays)
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
        # Cache the successful result
        _partial_summary_cache[cache_key] = summary_result
        return summary_result
    except Exception as e:
        print(f"Error parsing partial summary: {e}")
        # Don't cache errors - let user retry
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
            FROM claude_sessions.session_summaries
            WHERE session_id = %s
        """, (session_id,))
        row = cur.fetchone()
        if row:
            cur.close()
            conn.close()
            return dict(row)

    # Generate new summary
    summary_data = generate_session_summary(session_id)
    if not summary_data:
        cur.close()
        conn.close()
        return None

    # Save to database
    import json as json_module
    cur.execute("""
        INSERT INTO claude_sessions.session_summaries
            (session_id, summary, user_requests, completed_work, topics, model)
        VALUES (%s, %s, %s, %s, %s, %s)
        ON CONFLICT (session_id) DO UPDATE SET
            summary = EXCLUDED.summary,
            user_requests = EXCLUDED.user_requests,
            completed_work = EXCLUDED.completed_work,
            topics = EXCLUDED.topics,
            generated_at = NOW(),
            model = EXCLUDED.model
        RETURNING summary, user_requests, completed_work, topics, generated_at, model
    """, (
        session_id,
        summary_data["summary"],
        summary_data["user_requests"],
        summary_data["completed_work"],
        json_module.dumps(summary_data.get("topics", [])),
        MODEL_NAME
    ))

    result = cur.fetchone()
    conn.commit()
    cur.close()
    conn.close()

    return dict(result) if result else summary_data


def get_sessions_with_summaries(hours: float = 24, limit: int = 50) -> list[dict]:
    """Get sessions with their summaries for the card view."""
    conn = get_connection()
    cur = conn.cursor()

    from datetime import datetime, timedelta, timezone
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    cur.execute("""
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            s.end_time,
            COUNT(m.id) as total_messages,
            COUNT(*) FILTER (WHERE m.role = 'user') as user_messages,
            COUNT(*) FILTER (WHERE m.role = 'assistant') as assistant_messages,
            EXTRACT(EPOCH FROM (COALESCE(s.end_time, NOW()) - s.start_time))/60 as duration_mins,
            ss.summary,
            ss.user_requests,
            ss.completed_work,
            ss.topics
        FROM claude_sessions.sessions s
        LEFT JOIN claude_sessions.messages m ON s.session_id = m.session_id
        LEFT JOIN claude_sessions.session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= %s
        GROUP BY s.session_id, s.cwd, s.start_time, s.end_time,
                 ss.summary, ss.user_requests, ss.completed_work, ss.topics
        ORDER BY s.start_time DESC
        LIMIT %s
    """, (since, limit))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    sessions = []
    for row in rows:
        session = dict(row)
        # Clean up paths
        session['project'] = _normalize_path(session['cwd']) if session['cwd'] else ''
        session['is_active'] = session['end_time'] is None
        # Convert Decimal to float
        if session.get('duration_mins'):
            session['duration_mins'] = float(session['duration_mins'])
        sessions.append(session)

    return sessions
