"""Session context management for importance scoring.

SessionContext represents cached summary data for a session.
SessionContextManager handles creation, retrieval, and staleness detection.
"""
from dataclasses import dataclass
from datetime import datetime, timezone, timedelta
from typing import Optional
import json
import os

from ..queries import get_connection

# LiteLLM proxy configuration (same as summarizer.py)
LITELLM_BASE_URL = os.environ.get("LITELLM_BASE_URL", "http://localhost:4001")
LITELLM_API_KEY = os.environ.get("LITELLM_API_KEY", "sk-1234")  # Placeholder
MODEL_NAME = os.environ.get("SUMMARY_MODEL", "gemini-2.5-flash")


@dataclass
class SessionContext:
    """Cached context for a session, used to score message importance."""
    session_id: str
    summary: str
    completed_work: str
    topics: list[str]
    message_count: int
    generated_at: datetime

    def is_stale(self, current_message_count: int, threshold: int = 10) -> bool:
        """Context is stale if many new messages added since generation."""
        if self.message_count is None:
            return True
        return current_message_count - self.message_count > threshold


class SessionContextManager:
    """Manages creation and retrieval of session contexts."""

    SUMMARY_PROMPT = """Analyze this Claude Code conversation and provide a structured summary.

CONVERSATION:
{conversation}

Respond in exactly this JSON format (no markdown, just raw JSON):
{{
    "summary": "One paragraph overview of what happened in this session",
    "completed_work": "Bullet list of what was actually accomplished (use \\n for newlines)",
    "topics": ["topic1", "topic2", "topic3"]
}}

Be concise. Focus on the key points."""

    def __init__(self, staleness_days: float = 1.0):
        """Initialize manager.

        Args:
            staleness_days: How many days of inactivity before a session is
                            considered "done" and ready for summarization.
        """
        self.staleness_days = staleness_days

    def _generate_via_litellm(self, prompt: str) -> Optional[str]:
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

    def get_sessions_needing_context(self, limit: int = 50, since_days: float = None) -> list[str]:
        """Find sessions that need contexts (old enough, have unscored messages).

        Returns session_ids where:
        1. Last message is older than staleness_days
        2. Has at least one unscored message
        3. (Optional) Has messages within since_days
        """
        conn = get_connection()
        cur = conn.cursor()

        staleness_threshold = datetime.now(timezone.utc) - timedelta(days=self.staleness_days)

        if since_days is not None:
            # Filter to sessions with messages in the past since_days
            since_threshold = datetime.now(timezone.utc) - timedelta(days=since_days)
            cur.execute("""
                SELECT m.session_id, MAX(m.timestamp) as last_msg
                FROM claude_sessions.messages m
                WHERE m.importance_score IS NULL
                  AND m.timestamp >= %s
                  AND m.session_id IN (
                      SELECT session_id
                      FROM claude_sessions.messages
                      GROUP BY session_id
                      HAVING MAX(timestamp) < %s
                  )
                GROUP BY m.session_id
                ORDER BY last_msg DESC
                LIMIT %s
            """, (since_threshold, staleness_threshold, limit))
        else:
            cur.execute("""
                SELECT m.session_id, MAX(m.timestamp) as last_msg
                FROM claude_sessions.messages m
                WHERE m.importance_score IS NULL
                  AND m.session_id IN (
                      SELECT session_id
                      FROM claude_sessions.messages
                      GROUP BY session_id
                      HAVING MAX(timestamp) < %s
                  )
                GROUP BY m.session_id
                ORDER BY last_msg DESC
                LIMIT %s
            """, (staleness_threshold, limit))

        session_ids = [row['session_id'] for row in cur.fetchall()]
        cur.close()
        conn.close()
        return session_ids

    def get_stale_sessions(self, threshold: int = 10) -> list[dict]:
        """Get sessions where context is stale (many new messages since generation).

        Returns list of dicts with session_id, message_count_at_gen, current_count.
        """
        conn = get_connection()
        cur = conn.cursor()

        cur.execute("""
            SELECT
                ss.session_id,
                ss.message_count_at_gen,
                COUNT(m.id) as current_count
            FROM claude_sessions.session_summaries ss
            JOIN claude_sessions.messages m ON m.session_id = ss.session_id
            WHERE ss.message_count_at_gen IS NOT NULL
            GROUP BY ss.session_id, ss.message_count_at_gen
            HAVING COUNT(m.id) - ss.message_count_at_gen > %s
        """, (threshold,))

        rows = [dict(row) for row in cur.fetchall()]
        cur.close()
        conn.close()
        return rows

    def get_context(self, session_id: str) -> Optional[SessionContext]:
        """Get existing context from DB."""
        conn = get_connection()
        cur = conn.cursor()

        cur.execute("""
            SELECT summary, completed_work, topics, message_count_at_gen, generated_at
            FROM claude_sessions.session_summaries
            WHERE session_id = %s
        """, (session_id,))

        row = cur.fetchone()
        cur.close()
        conn.close()

        if not row:
            return None

        topics = row['topics'] if isinstance(row['topics'], list) else []
        return SessionContext(
            session_id=session_id,
            summary=row['summary'] or "",
            completed_work=row['completed_work'] or "",
            topics=topics,
            message_count=row['message_count_at_gen'] or 0,
            generated_at=row['generated_at'] or datetime.now(timezone.utc),
        )

    def _get_message_count(self, session_id: str) -> int:
        """Get current message count for a session."""
        conn = get_connection()
        cur = conn.cursor()
        cur.execute("""
            SELECT COUNT(*) as count FROM claude_sessions.messages WHERE session_id = %s
        """, (session_id,))
        row = cur.fetchone()
        cur.close()
        conn.close()
        return row['count'] if row else 0

    def create_context(self, session_id: str) -> Optional[SessionContext]:
        """Generate new context via LLM (expensive!).

        Creates a summary for the session and saves it to the database.
        """
        # Get all messages
        conn = get_connection()
        cur = conn.cursor()
        cur.execute("""
            SELECT role, content FROM claude_sessions.messages
            WHERE session_id = %s ORDER BY sequence_num
        """, (session_id,))
        messages = [dict(row) for row in cur.fetchall()]
        cur.close()
        conn.close()

        if not messages:
            return None

        message_count = len(messages)

        # Build conversation text
        conversation_parts = []
        for msg in messages:
            role = "USER" if msg['role'] == 'user' else "CLAUDE"
            content = msg['content'] or ""
            if len(content) > 2000:
                content = content[:2000] + "..."
            conversation_parts.append(f"{role}: {content}")

        conversation_text = "\n\n".join(conversation_parts)
        if len(conversation_text) > 50000:
            conversation_text = conversation_text[:50000] + "\n\n[TRUNCATED]"

        prompt = self.SUMMARY_PROMPT.format(conversation=conversation_text)

        # Generate summary via LLM
        text = self._generate_via_litellm(prompt)
        if not text:
            return None

        # Parse response
        try:
            text = text.strip()
            if text.startswith("```"):
                text = text.split("```")[1]
                if text.startswith("json"):
                    text = text[4:]
            text = text.strip()

            result = json.loads(text)
            summary = result.get("summary", "")
            completed_work = result.get("completed_work", "")
            topics = result.get("topics", [])
        except Exception as e:
            print(f"Error parsing summary: {e}")
            return None

        # Save to database
        now = datetime.now(timezone.utc)
        conn = get_connection()
        cur = conn.cursor()
        cur.execute("""
            INSERT INTO claude_sessions.session_summaries
                (session_id, summary, completed_work, topics, message_count_at_gen, generated_at, model)
            VALUES (%s, %s, %s, %s, %s, %s, %s)
            ON CONFLICT (session_id) DO UPDATE SET
                summary = EXCLUDED.summary,
                completed_work = EXCLUDED.completed_work,
                topics = EXCLUDED.topics,
                message_count_at_gen = EXCLUDED.message_count_at_gen,
                generated_at = EXCLUDED.generated_at,
                model = EXCLUDED.model
        """, (
            session_id,
            summary,
            completed_work,
            json.dumps(topics),
            message_count,
            now,
            MODEL_NAME
        ))
        conn.commit()
        cur.close()
        conn.close()

        return SessionContext(
            session_id=session_id,
            summary=summary,
            completed_work=completed_work,
            topics=topics,
            message_count=message_count,
            generated_at=now,
        )

    def get_or_create_context(self, session_id: str) -> tuple[Optional[SessionContext], bool]:
        """Get existing context or create if session is ready.

        Returns:
            (context, was_created) tuple. context is None if session not ready.
        """
        # Try to get existing
        context = self.get_context(session_id)
        if context:
            return context, False

        # Create new context
        context = self.create_context(session_id)
        if context:
            return context, True

        return None, False
