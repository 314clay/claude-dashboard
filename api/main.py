"""FastAPI wrapper for dashboard queries.

Provides REST endpoints for the Rust desktop app to fetch graph data.
"""
from fastapi import FastAPI, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from typing import Optional
import json
import uvicorn

# Import from local db module (self-contained)
from db.queries import (
    get_graph_data,
    get_sessions,
    get_overview_metrics,
    get_session_messages,
    get_session_summary,
    get_tool_usage,
    get_project_session_graph_data,
)
from db.summarizer import generate_partial_summary, get_or_create_summary
from db.importance.backfill import (
    backfill_importance_scores,
    get_importance_stats,
    score_single_session as score_session,
    rescore_sessions,
)

from project_detection import (
    get_project_summary,
    detect_project_for_session,
    backfill_detected_projects,
)
from db.semantic_filters import (
    get_all_filters,
    create_filter,
    delete_filter,
    get_filter_status,
)
from db.semantic_filter_scorer import (
    categorize_messages,
    get_filter_stats,
)
from db.health_ingest import (
    ingest_payload as health_ingest_payload,
    get_recent_sleep,
    get_health_stats,
)
from pydantic import BaseModel

app = FastAPI(
    title="Dashboard API",
    description="REST API for Claude Activity Dashboard",
    version="0.1.0",
)

# Allow requests from the Rust app
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["GET", "POST", "DELETE"],
    allow_headers=["*"],
)


# Pydantic models for request bodies
class SemanticFilterCreate(BaseModel):
    name: str
    query_text: str


class RescoreRequest(BaseModel):
    session_ids: list[str]


@app.get("/health")
def health():
    """Health check endpoint."""
    return {"status": "ok"}


@app.get("/graph")
def graph(
    hours: float = Query(default=24, description="Hours to look back"),
    session_id: Optional[str] = Query(default=None, description="Filter to specific session"),
):
    """Get graph nodes and edges for visualization.

    Returns nodes (messages) and edges (sequential links between messages).
    """
    nodes, edges = get_graph_data(hours, session_id)
    return {
        "nodes": nodes,
        "edges": edges,
        "node_count": len(nodes),
        "edge_count": len(edges),
    }


@app.get("/sessions")
def sessions(
    hours: float = Query(default=24, description="Hours to look back"),
    limit: int = Query(default=50, description="Max sessions to return"),
):
    """Get list of sessions with metadata."""
    df = get_sessions(hours, limit)
    if df.empty:
        return {"sessions": []}

    # Convert to list of dicts, handling datetime serialization
    records = df.to_dict(orient="records")
    for r in records:
        for k, v in r.items():
            if hasattr(v, 'isoformat'):
                r[k] = v.isoformat()

    return {"sessions": records}


@app.get("/metrics")
def metrics(
    hours: float = Query(default=24, description="Hours to look back"),
):
    """Get overview metrics (counts)."""
    return get_overview_metrics(hours)


@app.get("/session/{session_id}/messages")
def session_messages(session_id: str):
    """Get all messages for a specific session."""
    df = get_session_messages(session_id)
    if df.empty:
        return {"messages": []}

    records = df.to_dict(orient="records")
    for r in records:
        for k, v in r.items():
            if hasattr(v, 'isoformat'):
                r[k] = v.isoformat()

    return {"messages": records}


@app.get("/session/{session_id}/summary/partial")
def partial_summary(
    session_id: str,
    before_timestamp: str = Query(..., description="Generate summary up to this ISO timestamp"),
):
    """Generate a Gemini summary of the conversation up to a specific timestamp.

    Returns summary, completed work, unsuccessful attempts, and message counts.
    This call may take 2-5 seconds as it invokes Gemini.
    """
    result = generate_partial_summary(session_id, before_timestamp)
    if result is None:
        return {"error": "Failed to generate summary - no messages found"}
    return result


@app.get("/session/{session_id}/summary")
def session_summary(
    session_id: str,
    generate: bool = Query(default=False, description="Generate summary if it doesn't exist"),
):
    """Get the full session summary from the database.

    Returns summary, user_requests, completed_work, topics, detected_project.
    If generate=true, will create the summary via AI if it doesn't exist (may take 2-5 seconds).
    """
    # Fast path: just check database
    result = get_session_summary(session_id)
    if result is not None:
        return {"exists": True, "generated": False, **result}

    # No summary exists
    if not generate:
        return {"exists": False, "generated": False}

    # Generate new summary via AI
    result = get_or_create_summary(session_id)
    if result is None:
        return {"exists": False, "generated": False, "error": "Failed to generate summary"}

    # Convert datetime to ISO string if present
    if result.get('generated_at') and hasattr(result['generated_at'], 'isoformat'):
        result['generated_at'] = result['generated_at'].isoformat()

    return {"exists": True, "generated": True, **result}


@app.get("/tools")
def tools(
    hours: float = Query(default=24, description="Hours to look back"),
):
    """Get tool usage statistics."""
    df = get_tool_usage(hours)
    if df.empty:
        return {"tools": []}

    return {"tools": df.to_dict(orient="records")}


@app.get("/projects")
def projects():
    """Get list of detected projects with session counts."""
    return {"projects": get_project_summary()}


@app.get("/projects/graph")
def project_graph(
    hours: float = Query(default=720, description="Hours to look back (default 30 days)"),
):
    """Get hierarchical graph with projects as parent nodes.

    Returns project nodes connected to their session nodes.
    Projects come from detected_project or fallback to topics[0].
    """
    nodes, edges = get_project_session_graph_data(hours)
    return {
        "nodes": nodes,
        "edges": edges,
        "node_count": len(nodes),
        "edge_count": len(edges),
    }


@app.post("/projects/detect")
def detect_projects(
    dry_run: bool = Query(default=True, description="If true, don't update database"),
):
    """Run project detection on all sessions.

    Extracts project names from file paths in tool_usages.
    """
    return backfill_detected_projects(dry_run=dry_run)


# ==== Importance Scoring Endpoints ====

@app.get("/importance/stats")
def importance_stats():
    """Get statistics about importance scoring coverage."""
    return get_importance_stats()


@app.post("/importance/backfill")
def importance_backfill(
    max_sessions: int = Query(default=50, description="Max sessions to process"),
    staleness_days: float = Query(default=1.0, description="Days of inactivity before scoring"),
    batch_size: int = Query(default=100, description="Messages per LLM call"),
    parallel: int = Query(default=1, description="Parallel workers"),
    since_days: float = Query(default=None, description="Only process sessions with messages in past N days"),
):
    """Backfill importance scores for unscored messages.

    Calls Gemini to score messages on importance (0.0-1.0).
    This may take 2-5 seconds per session.
    """
    return backfill_importance_scores(max_sessions, staleness_days, batch_size, parallel, since_days)


@app.post("/importance/session/{session_id}")
def importance_score_session(
    session_id: str,
    batch_size: int = Query(default=25, description="Messages per LLM call"),
):
    """Score messages in a specific session.

    Calls Gemini to score unscored messages in the session.
    """
    return score_session(session_id, batch_size)


@app.post("/importance/rescore")
def importance_rescore(
    body: RescoreRequest,
    batch_size: int = Query(default=30, description="Messages per LLM call"),
):
    """Rescore importance for messages in specified sessions.

    Unlike /importance/session, this OVERWRITES existing scores atomically.
    If cancelled midway, no messages are left with NULL scores - they either
    keep their old score or get the new score.

    Body: { session_ids: ["uuid1", "uuid2", ...] }

    Returns: { sessions_processed, messages_rescored, errors }
    """
    return rescore_sessions(body.session_ids, batch_size)


def rescore_stream_generator(session_ids: list[str], batch_size: int):
    """Generator that yields SSE events for rescore progress."""
    from db.importance.context import SessionContextManager
    from db.importance.scorer import ImportanceScorer

    manager = SessionContextManager(staleness_days=0)
    scorer = ImportanceScorer()

    total = len(session_ids)
    messages_rescored = 0
    sessions_processed = 0
    errors = []

    for i, session_id in enumerate(session_ids):
        # Yield progress event
        progress = {
            "type": "progress",
            "current": i,
            "total": total,
            "session_id": session_id[:8],
            "messages_so_far": messages_rescored,
        }
        yield f"data: {json.dumps(progress)}\n\n"

        try:
            context, _ = manager.get_or_create_context(session_id)
            if context is None:
                errors.append(f"{session_id[:8]}: failed to create context")
                continue

            scores = scorer.score_session(
                session_id, context, batch_size=batch_size, force=True
            )
            sessions_processed += 1
            messages_rescored += len(scores)

        except Exception as e:
            errors.append(f"{session_id[:8]}: {str(e)}")

    # Yield final result
    result = {
        "type": "complete",
        "sessions_processed": sessions_processed,
        "messages_rescored": messages_rescored,
        "errors": errors,
    }
    yield f"data: {json.dumps(result)}\n\n"


@app.post("/importance/rescore/stream")
def importance_rescore_stream(
    body: RescoreRequest,
    batch_size: int = Query(default=30, description="Messages per LLM call"),
):
    """Rescore importance with streaming progress updates.

    Yields SSE events:
    - progress: { type: "progress", current, total, session_id, messages_so_far }
    - complete: { type: "complete", sessions_processed, messages_rescored, errors }
    """
    return StreamingResponse(
        rescore_stream_generator(body.session_ids, batch_size),
        media_type="text/event-stream",
    )


# ==== Semantic Filter Endpoints ====

@app.get("/semantic-filters")
def list_semantic_filters():
    """List all semantic filters with stats.

    Returns filters with total_scored and matches counts.
    """
    filters = get_all_filters()
    return {"filters": filters}


@app.post("/semantic-filters")
def create_semantic_filter(body: SemanticFilterCreate):
    """Create a new semantic filter.

    Body: { name, query_text }
    Returns the created filter.
    """
    try:
        filter_data = create_filter(body.name, body.query_text)
        return {"success": True, "filter": filter_data}
    except Exception as e:
        # Likely unique constraint violation on name
        return {"success": False, "error": str(e)}


@app.delete("/semantic-filters/{filter_id}")
def delete_semantic_filter(filter_id: int):
    """Delete a semantic filter and its results.

    Returns success status.
    """
    deleted = delete_filter(filter_id)
    if deleted:
        return {"success": True, "deleted": filter_id}
    else:
        return {"success": False, "error": "Filter not found"}


@app.get("/semantic-filters/{filter_id}/status")
def semantic_filter_status(filter_id: int):
    """Get scoring progress for a specific filter.

    Returns: { filter_id, name, total, scored, pending, matches }
    """
    status = get_filter_status(filter_id)
    if status is None:
        return {"error": "Filter not found"}
    return status


@app.post("/semantic-filters/{filter_id}/categorize")
def categorize_filter_messages(
    filter_id: int,
    batch_size: int = Query(default=50, description="Messages per LLM call (50-100 recommended)"),
    max_messages: int = Query(default=5000, description="Maximum messages to process"),
):
    """Trigger categorization for a semantic filter.

    Scores unscored messages against all active filters in batches.
    Uses Gemini to determine which messages match the filter criteria.

    This may take several seconds depending on the number of unscored messages.

    Returns: { filter_id, scored, matches, batches_processed, errors }
    """
    return categorize_messages(filter_id, batch_size, max_messages)


@app.get("/semantic-filters/stats")
def semantic_filter_stats():
    """Get statistics about semantic filter scoring coverage.

    Returns: { total_messages, filters: [{ id, name, scored_count, match_count }] }
    """
    return get_filter_stats()


# ==== Health Auto Export Endpoints ====

@app.post("/health/ingest")
async def health_ingest(payload: dict):
    """Ingest health data from Health Auto Export iOS app.

    Accepts webhook payloads from Health Auto Export and stores them
    in the health schema (sleep_analysis, metrics tables).

    Expected payload format:
    {
        "data": {
            "metrics": [
                {"name": "sleep_analysis", "data": [...]},
                {"name": "heart_rate", "units": "bpm", "data": [...]}
            ],
            "workouts": [...]
        }
    }

    Returns: { status, ingest_id, sleep_records, metric_records }
    """
    try:
        result = health_ingest_payload(payload)
        return result
    except Exception as e:
        return {"status": "error", "message": str(e)}


@app.get("/health/sleep")
def health_sleep(days: int = 7):
    """Get recent sleep data.

    Returns sleep records from the last N days.
    """
    records = get_recent_sleep(days)
    # Convert datetimes to ISO strings
    for r in records:
        for k, v in r.items():
            if hasattr(v, 'isoformat'):
                r[k] = v.isoformat()
    return {"records": records, "count": len(records)}


@app.get("/health/stats")
def health_stats():
    """Get health data statistics.

    Returns counts of ingests, sleep records, and metrics.
    """
    return get_health_stats()


if __name__ == "__main__":
    # Bind to 0.0.0.0 to allow access via Tailscale
    # Port 10800 for dashboard API / health ingest
    uvicorn.run(app, host="0.0.0.0", port=10800)
