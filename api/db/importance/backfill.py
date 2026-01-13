"""Backfill orchestration for importance scoring.

Two-phase approach:
1. Find sessions that are "done" (no activity for staleness_days)
2. For each: get/create context (expensive if creating), then score messages (cheap)
"""
from concurrent.futures import ThreadPoolExecutor, as_completed
from ..queries import get_connection
from .context import SessionContextManager
from .scorer import ImportanceScorer


def get_importance_stats() -> dict:
    """Get statistics about importance scoring coverage."""
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT
            COUNT(*) as total_messages,
            COUNT(importance_score) as scored_messages,
            COUNT(*) - COUNT(importance_score) as unscored_messages,
            AVG(importance_score) as avg_score,
            COUNT(DISTINCT session_id) FILTER (WHERE importance_score IS NULL) as sessions_with_unscored
        FROM claude_sessions.messages
    """)

    row = cur.fetchone()
    cur.close()
    conn.close()

    return {
        "total_messages": row['total_messages'],
        "scored_messages": row['scored_messages'],
        "unscored_messages": row['unscored_messages'],
        "avg_score": float(row['avg_score']) if row['avg_score'] else None,
        "sessions_with_unscored": row['sessions_with_unscored']
    }


def _process_single_session(
    session_id: str,
    staleness_days: float,
    batch_size: int
) -> dict:
    """Process a single session. Thread-safe.

    Returns dict with session_id, context_created, messages_scored, error.
    """
    manager = SessionContextManager(staleness_days=staleness_days)
    scorer = ImportanceScorer()

    try:
        context, was_created = manager.get_or_create_context(session_id)
        if context is None:
            return {"session_id": session_id, "error": "failed to create context"}

        scores = scorer.score_session(session_id, context, batch_size=batch_size)
        # score_session already saves scores internally, so use len(scores)

        return {
            "session_id": session_id,
            "context_created": was_created,
            "messages_scored": len(scores),
            "error": None
        }
    except Exception as e:
        return {"session_id": session_id, "error": str(e)}


def backfill_importance_scores(
    max_sessions: int = 50,
    staleness_days: float = 1.0,
    batch_size: int = 100,
    parallel: int = 1,
    since_days: float = None
) -> dict:
    """Main backfill function. Two-phase approach:

    1. Find sessions that are "done" (no activity for staleness_days)
    2. For each:
       a. Get or create SessionContext (expensive if creating)
       b. Score all unscored messages using context (cheap per message)

    Args:
        max_sessions: Maximum number of sessions to process
        staleness_days: Days of inactivity before session is considered "done"
        batch_size: Messages per LLM scoring call
        parallel: Number of parallel workers (1 = sequential)
        since_days: Only process sessions with messages in the past N days (None = all)

    Returns:
        Summary dict with sessions_processed, messages_scored, contexts_created, errors
    """
    manager = SessionContextManager(staleness_days=staleness_days)

    results = {
        "sessions_processed": 0,
        "messages_scored": 0,
        "contexts_created": 0,
        "contexts_reused": 0,
        "errors": []
    }

    # Get sessions that need scoring (old enough, have unscored messages)
    session_ids = manager.get_sessions_needing_context(limit=max_sessions, since_days=since_days)

    if parallel > 1:
        # Parallel execution
        with ThreadPoolExecutor(max_workers=parallel) as executor:
            futures = {
                executor.submit(
                    _process_single_session,
                    session_id,
                    staleness_days,
                    batch_size
                ): session_id
                for session_id in session_ids
            }

            for future in as_completed(futures):
                result = future.result()
                if result.get("error"):
                    results["errors"].append(f"{result['session_id'][:8]}: {result['error']}")
                else:
                    results["sessions_processed"] += 1
                    results["messages_scored"] += result.get("messages_scored", 0)
                    if result.get("context_created"):
                        results["contexts_created"] += 1
                    else:
                        results["contexts_reused"] += 1
    else:
        # Sequential execution (original behavior)
        scorer = ImportanceScorer()
        for session_id in session_ids:
            try:
                context, was_created = manager.get_or_create_context(session_id)
                if context is None:
                    results["errors"].append(f"{session_id[:8]}: failed to create context")
                    continue

                if was_created:
                    results["contexts_created"] += 1
                else:
                    results["contexts_reused"] += 1

                scores = scorer.score_session(session_id, context, batch_size=batch_size)
                # score_session already saves scores internally

                results["sessions_processed"] += 1
                results["messages_scored"] += len(scores)

            except Exception as e:
                results["errors"].append(f"{session_id[:8]}: {str(e)}")

    return results


def score_single_session(session_id: str, batch_size: int = 100) -> dict:
    """Score messages in a specific session.

    Gets or creates context, then scores all unscored messages.

    Returns:
        dict with session_id, context_created, messages_scored, status
    """
    manager = SessionContextManager(staleness_days=0)  # Don't check staleness
    scorer = ImportanceScorer()

    context, was_created = manager.get_or_create_context(session_id)
    if context is None:
        return {
            "session_id": session_id,
            "status": "failed",
            "error": "Could not create context"
        }

    scores = scorer.score_session(session_id, context, batch_size=batch_size)
    saved = scorer.save_scores(scores)

    return {
        "session_id": session_id,
        "context_created": was_created,
        "messages_scored": saved,
        "status": "ok"
    }


def rescore_sessions(session_ids: list[str], batch_size: int = 30) -> dict:
    """Rescore importance for messages in specified sessions.

    Unlike score_single_session, this overwrites existing scores atomically.
    If cancelled midway, no messages are left with NULL scores - they either
    have their old score or new score.

    Args:
        session_ids: List of session UUIDs to rescore
        batch_size: Messages per LLM call

    Returns:
        dict with sessions_processed, messages_rescored, errors
    """
    manager = SessionContextManager(staleness_days=0)
    scorer = ImportanceScorer()

    results = {
        "sessions_processed": 0,
        "messages_rescored": 0,
        "errors": []
    }

    for session_id in session_ids:
        try:
            # Get or create context (reuse if available)
            context, _ = manager.get_or_create_context(session_id)
            if context is None:
                results["errors"].append(f"{session_id[:8]}: failed to create context")
                continue

            # Rescore with force=True (atomic overwrites, no clearing)
            scores = scorer.score_session(
                session_id, context, batch_size=batch_size, force=True
            )

            results["sessions_processed"] += 1
            results["messages_rescored"] += len(scores)

        except Exception as e:
            results["errors"].append(f"{session_id[:8]}: {str(e)}")

    return results
