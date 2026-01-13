"""Importance scorer using session context.

Scores messages in batch, using the session summary as context
for relative importance within the conversation.
"""
import json
import os
from datetime import datetime, timezone
from typing import Optional

from ..queries import get_connection
from .context import SessionContext

# LiteLLM proxy configuration
LITELLM_BASE_URL = os.environ.get("LITELLM_BASE_URL", "http://localhost:4001")
LITELLM_API_KEY = os.environ.get("LITELLM_API_KEY", "sk-litellm-master-key")
MODEL_NAME = os.environ.get("SUMMARY_MODEL", "gemini-2.5-flash")


class ImportanceScorer:
    """Scores messages in batch using session context."""

    SCORING_PROMPT = """You are scoring the importance of messages in a Claude Code conversation.

SESSION CONTEXT:
{summary}

Completed work in this session:
{completed_work}

Topics: {topics}

SCORING CRITERIA (0.0 to 1.0) - Score based on DECISIONS made, not actions taken:

- Major Decisions (0.8-1.0): Architectural choices, technology picks, design direction, "let's use X approach"
- Minor Decisions (0.6-0.8): Implementation choices, API design, tradeoff resolutions
- Task Definitions (0.5-0.7): Defining what to build, scoping work, setting requirements
- Context (0.3-0.5): Background info, explanations, clarifications that inform decisions
- Execution (0.1-0.3): Running commands, building, testing, routine operations - NO decision made
- Filler (0.0-0.2): "thanks", "got it", "ok", acknowledgments

KEY: A message with code or commands is LOW importance unless it represents a DECISION about what/how to build. "Run the build" = 0.2 (execution). "Let's add caching with Redis" = 0.8 (decision).

MESSAGES TO SCORE:
{messages}

Respond with JSON only, no markdown code blocks:
{{"scores": [{{"id": <message_id>, "score": <0.0-1.0>, "reason": "<brief reason, max 50 chars>"}}]}}"""

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
                max_tokens=8192,
            )
            return response.choices[0].message.content
        except Exception as e:
            print(f"LiteLLM error: {e}")
            return None

    def get_unscored_messages(self, session_id: str, limit: int = 30, force: bool = False) -> list[dict]:
        """Get messages for a session that need scoring.

        Args:
            session_id: Session to get messages from
            limit: Max messages to return
            force: If True, get ALL messages (for rescoring). If False, only unscored.
        """
        conn = get_connection()
        cur = conn.cursor()

        if force:
            # Get all messages regardless of current score
            cur.execute("""
                SELECT id, role, content, sequence_num
                FROM claude_sessions.messages
                WHERE session_id = %s
                ORDER BY sequence_num
                LIMIT %s
            """, (session_id, limit))
        else:
            cur.execute("""
                SELECT id, role, content, sequence_num
                FROM claude_sessions.messages
                WHERE session_id = %s AND importance_score IS NULL
                ORDER BY sequence_num
                LIMIT %s
            """, (session_id, limit))

        messages = [dict(row) for row in cur.fetchall()]
        cur.close()
        conn.close()
        return messages

    def score_batch(
        self,
        messages: list[dict],
        context: SessionContext
    ) -> dict[int, tuple[float, str]]:
        """Score all messages in one LLM call with summary context.

        Args:
            messages: List of message dicts with id, role, content
            context: SessionContext with summary and completed_work

        Returns:
            dict mapping message_id -> (score, reason)
        """
        if not messages:
            return {}

        # Format messages for prompt
        formatted = []
        for msg in messages:
            content = msg['content'] or ""
            if len(content) > 1500:
                content = content[:1500] + "...[truncated]"
            role = "USER" if msg['role'] == 'user' else "CLAUDE"
            formatted.append(f"[{msg['id']}] {role}: {content}")

        messages_text = "\n\n".join(formatted)
        if len(messages_text) > 40000:
            messages_text = messages_text[:40000] + "\n\n[TRUNCATED]"

        prompt = self.SCORING_PROMPT.format(
            summary=context.summary,
            completed_work=context.completed_work,
            topics=", ".join(context.topics) if context.topics else "general",
            messages=messages_text,
        )

        response = self._generate_via_litellm(prompt)
        if not response:
            return {}

        # Parse response
        try:
            text = response.strip()
            if text.startswith("```"):
                text = text.split("```")[1]
                if text.startswith("json"):
                    text = text[4:]
            text = text.strip()

            # Fix trailing commas (common LLM mistake)
            import re
            text = re.sub(r',(\s*[}\]])', r'\1', text)

            result = json.loads(text)
            scores = {}
            for s in result.get('scores', []):
                msg_id = s.get('id')
                score = s.get('score', 0.5)
                reason = s.get('reason', '')[:255]
                if msg_id is not None:
                    scores[int(msg_id)] = (float(score), reason)
            return scores
        except Exception as e:
            print(f"Error parsing importance scores: {e}")
            print(f"Response was: {response[:500]}...")
            return {}

    def score_session(
        self,
        session_id: str,
        context: SessionContext,
        batch_size: int = 30,
        force: bool = False
    ) -> dict[int, tuple[float, str]]:
        """Get messages and score them in batches.

        Args:
            session_id: Session to score
            context: SessionContext with summary
            batch_size: Max messages per LLM call
            force: If True, rescore ALL messages (overwrites existing). If False, only unscored.

        Returns:
            dict mapping message_id -> (score, reason)
        """
        all_scores = {}
        scored_ids = set()  # Track which IDs we've processed (for force mode)

        while True:
            messages = self.get_unscored_messages(session_id, limit=batch_size, force=force)

            # In force mode, skip messages we've already scored this run
            if force:
                messages = [m for m in messages if m['id'] not in scored_ids]

            if not messages:
                break

            batch_scores = self.score_batch(messages, context)
            if not batch_scores:
                break  # LLM failed, stop retrying

            # Save scores immediately (atomic overwrite in force mode)
            saved = self.save_scores(batch_scores, force=force)
            all_scores.update(batch_scores)
            scored_ids.update(batch_scores.keys())

            # If we got fewer than batch_size, we're done
            if len(messages) < batch_size:
                break

        return all_scores

    def save_scores(self, scores: dict[int, tuple[float, str]], force: bool = False) -> int:
        """Save importance scores to database.

        Args:
            scores: dict mapping message_id -> (score, reason)
            force: If True, overwrite existing scores. If False, only update NULL scores.

        Returns: number of rows updated
        """
        if not scores:
            return 0

        conn = get_connection()
        cur = conn.cursor()

        now = datetime.now(timezone.utc)
        updated = 0

        for msg_id, (score, reason) in scores.items():
            if force:
                # Overwrite regardless of current value (atomic update)
                cur.execute("""
                    UPDATE claude_sessions.messages
                    SET importance_score = %s,
                        importance_reason = %s,
                        importance_scored_at = %s
                    WHERE id = %s
                """, (score, reason if reason else None, now, msg_id))
            else:
                cur.execute("""
                    UPDATE claude_sessions.messages
                    SET importance_score = %s,
                        importance_reason = %s,
                        importance_scored_at = %s
                    WHERE id = %s AND importance_score IS NULL
                """, (score, reason if reason else None, now, msg_id))
            updated += cur.rowcount

        conn.commit()
        cur.close()
        conn.close()
        return updated
