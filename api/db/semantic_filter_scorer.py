"""Semantic filter scorer using LLM-based categorization.

Categorizes messages against user-defined semantic filters.
Each filter has a query_text that describes what content should match.
"""
import json
import os
import re
from datetime import datetime, timezone
from typing import Optional

from .queries import get_connection
from . import llm


class SemanticFilterScorer:
    """Categorizes messages against semantic filters in batch."""

    CATEGORIZATION_PROMPT = """You are categorizing messages from Claude Code conversations.

FILTERS TO MATCH AGAINST (use the numeric ID in your response):
{filters}

For each message, determine which filters it matches. A message matches a filter if its content
satisfies the filter's query criteria. Be inclusive but accurate.

MESSAGES TO CATEGORIZE:
{messages}

Respond with JSON only, no markdown code blocks:
{{"results": [{{"id": <message_id_integer>, "matches": [<filter_id_integer>, ...]}}]}}

IMPORTANT:
- "id" must be the numeric message ID (e.g., 13397)
- "matches" must be a list of numeric filter IDs (e.g., [1, 2] not ["Filter 1", "Filter 2"])
- If no filters match, use an empty list: []"""

    def __init__(self):
        pass

    def _generate(self, prompt: str) -> Optional[str]:
        """Generate text via the configured LLM provider."""
        if not llm.is_available():
            print("LLM not configured - semantic filter scoring disabled")
            return None
        return llm.complete(
            messages=[{"role": "user", "content": prompt}],
            max_tokens=8192,
        )

    def get_active_filters(self) -> list[dict]:
        """Get all active semantic filters from database.

        Returns:
            List of dicts: [{"id": int, "name": str, "query_text": str}, ...]
        """
        conn = get_connection()
        cur = conn.cursor()

        cur.execute("""
            SELECT id, name, query_text
            FROM semantic_filters
            WHERE is_active = 1
            ORDER BY id
        """)

        filters = [dict(row) for row in cur.fetchall()]
        cur.close()
        conn.close()
        return filters

    def get_unscored_messages_for_filter(
        self,
        filter_id: int,
        limit: int = 100
    ) -> list[dict]:
        """Get messages not yet scored for a specific filter.

        Args:
            filter_id: The filter to check against
            limit: Max messages to return

        Returns:
            List of message dicts: [{"id": int, "role": str, "content": str}, ...]
        """
        conn = get_connection()
        cur = conn.cursor()

        cur.execute("""
            SELECT m.id, m.role, m.content
            FROM messages m
            WHERE NOT EXISTS (
                SELECT 1 FROM semantic_filter_results sfr
                WHERE sfr.message_id = m.id AND sfr.filter_id = ?
            )
            ORDER BY m.timestamp DESC
            LIMIT ?
        """, (filter_id, limit))

        messages = [dict(row) for row in cur.fetchall()]
        cur.close()
        conn.close()
        return messages

    def score_batch(
        self,
        messages: list[dict],
        filters: list[dict]
    ) -> dict[int, list[int]]:
        """Score a batch of messages against all filters in one LLM call.

        Args:
            messages: List of message dicts with id, role, content
            filters: List of filter dicts with id, name, query_text

        Returns:
            dict mapping message_id -> list of matching filter_ids
        """
        if not messages or not filters:
            return {}

        # Format filters for prompt
        filters_text = "\n".join([
            f"[Filter {f['id']}] {f['name']}: {f['query_text']}"
            for f in filters
        ])

        # Format messages for prompt (truncate content)
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

        prompt = self.CATEGORIZATION_PROMPT.format(
            filters=filters_text,
            messages=messages_text,
        )

        response = self._generate(prompt)
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
            text = re.sub(r',(\s*[}\]])', r'\1', text)

            result = json.loads(text)
            matches = {}
            for r in result.get('results', []):
                msg_id = r.get('id')
                matching_filters = r.get('matches', [])
                if msg_id is not None:
                    parsed_filters = []
                    for f in matching_filters:
                        if isinstance(f, int):
                            parsed_filters.append(f)
                        elif isinstance(f, str):
                            match = re.match(r'Filter\s*(\d+)', f, re.IGNORECASE)
                            if match:
                                parsed_filters.append(int(match.group(1)))
                            elif f.isdigit():
                                parsed_filters.append(int(f))
                    matches[int(msg_id)] = parsed_filters
            return matches
        except Exception as e:
            print(f"Error parsing categorization response: {e}")
            print(f"Response was: {response[:500]}...")
            return {}

    def save_results(
        self,
        results: dict[int, list[int]],
        all_filter_ids: list[int]
    ) -> int:
        """Save categorization results to database.

        For each message, saves a result for EVERY filter (matches=true/false).

        Args:
            results: dict mapping message_id -> list of matching filter_ids
            all_filter_ids: list of all filter IDs that were evaluated

        Returns:
            Number of rows inserted
        """
        if not results:
            return 0

        conn = get_connection()
        cur = conn.cursor()

        now = datetime.now(timezone.utc).isoformat()
        inserted = 0

        for msg_id, matching_filters in results.items():
            matching_set = set(matching_filters)

            for filter_id in all_filter_ids:
                matches = 1 if filter_id in matching_set else 0
                confidence = 1.0 if matches else 0.0

                cur.execute("""
                    INSERT INTO semantic_filter_results
                    (filter_id, message_id, matches, confidence, scored_at)
                    VALUES (?, ?, ?, ?, ?)
                    ON CONFLICT (filter_id, message_id)
                    DO UPDATE SET matches = excluded.matches,
                                  confidence = excluded.confidence,
                                  scored_at = excluded.scored_at
                """, (filter_id, msg_id, matches, confidence, now))
                inserted += cur.rowcount

        conn.commit()
        cur.close()
        conn.close()
        return inserted


def categorize_messages(
    filter_id: int,
    batch_size: int = 50,
    max_messages: int = 5000
) -> dict:
    """Main function to categorize messages for a filter.

    Gets all active filters, finds unscored messages for the specified filter,
    batches and scores them, and saves results.

    Args:
        filter_id: The filter ID to focus on (determines which messages need scoring)
        batch_size: Messages per LLM call (50-100 recommended)
        max_messages: Maximum total messages to process

    Returns:
        dict with scored, matches, errors
    """
    scorer = SemanticFilterScorer()

    results = {
        "filter_id": filter_id,
        "scored": 0,
        "matches": 0,
        "batches_processed": 0,
        "errors": []
    }

    filters = scorer.get_active_filters()
    if not filters:
        results["errors"].append("No active filters found")
        return results

    filter_ids = [f['id'] for f in filters]
    if filter_id not in filter_ids:
        results["errors"].append(f"Filter {filter_id} not found or inactive")
        return results

    all_filter_ids = filter_ids
    messages_processed = 0

    while messages_processed < max_messages:
        messages = scorer.get_unscored_messages_for_filter(
            filter_id,
            limit=batch_size
        )

        if not messages:
            break

        batch_results = scorer.score_batch(messages, filters)

        if not batch_results:
            results["errors"].append(f"Batch {results['batches_processed']} failed to score")
            break

        saved = scorer.save_results(batch_results, all_filter_ids)

        for msg_id, matching_filters in batch_results.items():
            if filter_id in matching_filters:
                results["matches"] += 1

        results["scored"] += len(batch_results)
        results["batches_processed"] += 1
        messages_processed += len(messages)

        if len(messages) < batch_size:
            break

    return results


def get_filter_stats() -> dict:
    """Get statistics about semantic filter scoring.

    Returns:
        dict with filter coverage and match stats
    """
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT
            sf.id,
            sf.name,
            sf.query_text,
            sf.is_active,
            COUNT(sfr.message_id) as scored_count,
            SUM(CASE WHEN sfr.matches = 1 THEN 1 ELSE 0 END) as match_count
        FROM semantic_filters sf
        LEFT JOIN semantic_filter_results sfr ON sf.id = sfr.filter_id
        GROUP BY sf.id, sf.name, sf.query_text, sf.is_active
        ORDER BY sf.id
    """)

    filters = [dict(row) for row in cur.fetchall()]

    cur.execute("SELECT COUNT(*) as total FROM messages")
    total_messages = cur.fetchone()['total']

    cur.close()
    conn.close()

    return {
        "total_messages": total_messages,
        "filters": filters
    }
