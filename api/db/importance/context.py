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
from .. import llm


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

    def _generate(self, prompt: str) -> Optional[str]:
        """Generate text via the configured LLM provider."""
        if not llm.is_available():
            print("LLM not configured - context creation disabled")
            return None
        return llm.complete(
            messages=[{"role": "user", "content": prompt}],
            max_tokens=2048,
        )

    def get_sessions_needing_context(self, limit: int = 50, since_days: float = None) -> list[str]:
        """Find sessions that need contexts (old enough, have unscored messages).

        Returns session_ids where:
        1. Last message is older than staleness_days
        2. Has at least one unscored message
        3. (Optional) Has messages within since_days
        """
        conn = get_connection()
        cur = conn.cursor()

        staleness_threshold = (datetime.now(timezone.utc) - timedelta(days=self.staleness_days)).isoformat()

        if since_days is not None:
            since_threshold = (datetime.now(timezone.utc) - timedelta(days=since_days)).isoformat()
            cur.execute("""
                SELECT m.session_id, MAX(m.timestamp) as last_msg
                FROM messages m
                WHERE m.importance_score IS NULL
                  AND m.timestamp >= ?
                  AND m.session_id IN (
                      SELECT session_id
                      FROM messages
                      GROUP BY session_id
                      HAVING MAX(timestamp) < ?
                  )
                GROUP BY m.session_id
                ORDER BY last_msg DESC
                LIMIT ?
            """, (since_threshold, staleness_threshold, limit))
        else:
            cur.execute("""
                SELECT m.session_id, MAX(m.timestamp) as last_msg
                FROM messages m
                WHERE m.importance_score IS NULL
                  AND m.session_id IN (
                      SELECT session_id
                      FROM messages
                      GROUP BY session_id
                      HAVING MAX(timestamp) < ?
                  )
                GROUP BY m.session_id
                ORDER BY last_msg DESC
                LIMIT ?
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
            FROM session_summaries ss
            JOIN messages m ON m.session_id = ss.session_id
            WHERE ss.message_count_at_gen IS NOT NULL
            GROUP BY ss.session_id, ss.message_count_at_gen
            HAVING COUNT(m.id) - ss.message_count_at_gen > ?
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
            FROM session_summaries
            WHERE session_id = ?
        """, (session_id,))

        row = cur.fetchone()
        cur.close()
        conn.close()

        if not row:
            return None

        raw_topics = row['topics']
        if isinstance(raw_topics, str):
            try:
                topics = json.loads(raw_topics)
            except (json.JSONDecodeError, TypeError):
                topics = []
        elif isinstance(raw_topics, list):
            topics = raw_topics
        else:
            topics = []

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
            SELECT COUNT(*) as count FROM messages WHERE session_id = ?
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
            SELECT role, content FROM messages
            WHERE session_id = ? ORDER BY sequence_num
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
        text = self._generate(prompt)
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
        now = datetime.now(timezone.utc).isoformat()
        conn = get_connection()
        cur = conn.cursor()
        cur.execute("""
            INSERT INTO session_summaries
                (session_id, summary, completed_work, topics, message_count_at_gen, generated_at, model)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT (session_id) DO UPDATE SET
                summary = excluded.summary,
                completed_work = excluded.completed_work,
                topics = excluded.topics,
                message_count_at_gen = excluded.message_count_at_gen,
                generated_at = excluded.generated_at,
                model = excluded.model
        """, (
            session_id,
            summary,
            completed_work,
            json.dumps(topics),
            message_count,
            now,
            llm.get_provider() or "unknown"
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
