"""FastAPI wrapper for dashboard queries.

Provides REST endpoints for the Rust desktop app to fetch graph data.
"""
from fastapi import FastAPI, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from typing import Optional
import json
import uvicorn
from pathlib import Path as FilePath

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
from db.summarizer import generate_partial_summary, get_or_create_summary, generate_neighborhood_summary
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
    categorize_messages_visible,
    get_filter_stats,
)
from db.rule_filter_scorer import score_rule_filter
from db.embeddings import (
    get_embedding_stats,
    generate_embeddings,
    search_by_query,
    compute_proximity_edges,
)
from db.mail import get_mail_network
# Commented out - health_ingest module not present
# from db.health_ingest import (
#     ingest_payload as health_ingest_payload,
#     get_recent_sleep,
#     get_health_stats,
# )
import subprocess
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
    filter_type: str = 'semantic'


class RescoreRequest(BaseModel):
    session_ids: list[str]


class NeighborhoodSummaryRequest(BaseModel):
    message_ids: list[str]


class SimilaritySearchRequest(BaseModel):
    query_text: str


class CategorizeVisibleRequest(BaseModel):
    message_ids: list[int]


class ProximityEdgesRequest(BaseModel):
    query_text: str
    delta: float = 0.1
    max_edges: int = 100_000
    max_neighbors: int = 0


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
    rows = get_sessions(hours, limit)
    return {"sessions": rows}


@app.get("/metrics")
def metrics(
    hours: float = Query(default=24, description="Hours to look back"),
):
    """Get overview metrics (counts)."""
    return get_overview_metrics(hours)


@app.get("/session/{session_id}/messages")
def session_messages(session_id: str):
    """Get all messages for a specific session."""
    rows = get_session_messages(session_id)
    return {"messages": rows}


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


@app.post("/summary/neighborhood")
def neighborhood_summary(body: NeighborhoodSummaryRequest):
    """Generate an AI summary covering a node and its direct graph neighbors.

    Body: { "message_ids": ["123", "456", ...] }
    Returns: { summary, themes, node_count, session_count }
    """
    try:
        int_ids = [int(mid) for mid in body.message_ids]
    except ValueError:
        return {"error": "All message_ids must be numeric strings"}

    if not int_ids:
        return {"error": "message_ids must not be empty"}

    result = generate_neighborhood_summary(int_ids)
    if result is None:
        return {"error": "Failed to generate neighborhood summary"}
    return result


@app.get("/tools")
def tools(
    hours: float = Query(default=24, description="Hours to look back"),
):
    """Get tool usage statistics."""
    rows = get_tool_usage(hours)
    return {"tools": rows}


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

    Body: { name, query_text, filter_type? }
    Returns the created filter. Rule filters are auto-scored immediately.
    """
    try:
        filter_data = create_filter(body.name, body.query_text, body.filter_type)

        # Auto-score rule filters immediately
        if body.filter_type == 'rule':
            result = score_rule_filter(filter_data['id'], body.query_text)
            # Update the returned filter with fresh counts
            filter_data['total_scored'] = result['scored']
            filter_data['matches'] = result['matches']

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
    max_concurrent: int = Query(default=4, ge=1, le=8, description="Max parallel LLM calls (1-8)"),
):
    """Trigger categorization for a semantic filter.

    For rule-based filters, scores instantly without LLM.
    For semantic filters, scores unscored messages in parallel batches via LLM.

    Returns: { filter_id, scored, matches, ... }
    """
    # Check if this is a rule filter
    status = get_filter_status(filter_id)
    if status and status.get('filter_type') == 'rule':
        return score_rule_filter(filter_id, status['query_text'])

    return categorize_messages(filter_id, batch_size, max_messages, max_concurrent)


@app.post("/semantic-filters/{filter_id}/categorize-visible")
def categorize_filter_messages_visible(
    filter_id: int,
    body: CategorizeVisibleRequest,
    batch_size: int = Query(default=50, description="Messages per LLM call (50-100 recommended)"),
    max_concurrent: int = Query(default=4, ge=1, le=8, description="Max parallel LLM calls (1-8)"),
):
    """Trigger categorization for a semantic filter on specific message IDs only.

    Scores only the provided messages (skipping already-scored ones) against all
    active filters in parallel batches. Useful for scoring only currently visible
    graph nodes.

    Body: { message_ids: [1, 2, 3, ...] }
    Returns: { filter_id, scored, matches, batches_processed, errors }
    """
    return categorize_messages_visible(filter_id, body.message_ids, batch_size, max_concurrent)


@app.get("/semantic-filters/stats")
def semantic_filter_stats():
    """Get statistics about semantic filter scoring coverage.

    Returns: { total_messages, filters: [{ id, name, scored_count, match_count }] }
    """
    return get_filter_stats()


# ==== Embedding / Similarity Search Endpoints ====

@app.get("/embeddings/stats")
def embedding_stats():
    """Get embedding coverage statistics.

    Returns: { total, embedded, unembedded, model }
    """
    return get_embedding_stats()


@app.post("/embeddings/generate")
def embedding_generate(
    batch_size: int = Query(default=100, description="Texts per API call"),
    max_messages: int = Query(default=1000, description="Max messages to embed"),
):
    """Generate embeddings for unembedded messages.

    Calls OpenAI text-embedding-3-small (or Google equivalent).
    Returns: { generated, model, dimensions, errors }
    """
    return generate_embeddings(batch_size, max_messages)


class GenerateVisibleRequest(BaseModel):
    message_ids: list[int]
    batch_size: int = 100
    max_messages: int = 50000


@app.post("/embeddings/generate-visible")
def embedding_generate_visible(body: GenerateVisibleRequest):
    """Generate embeddings only for specific message IDs.

    Body: { message_ids: [1, 2, 3], batch_size: 100, max_messages: 50000 }
    Returns: { generated, model, dimensions, errors }
    """
    return generate_embeddings(body.batch_size, body.max_messages, message_ids=body.message_ids)


@app.post("/embeddings/search")
def embedding_search(body: SimilaritySearchRequest):
    """Search messages by semantic similarity.

    Body: { query_text: "frustrated" }
    Returns: { scores: { message_id: float } } for ALL embedded messages.
    """
    scores = search_by_query(body.query_text)
    # Convert int keys to strings for JSON serialization
    return {"scores": {str(k): v for k, v in scores.items()}}


@app.post("/embeddings/proximity-edges")
def embedding_proximity_edges(body: ProximityEdgesRequest):
    """Compute score-proximity edges between embedded messages.

    Given a query phrase, scores all nodes by similarity to that phrase,
    then links nodes whose scores are within `delta` of each other.

    Body: { query_text: "...", delta: 0.1, max_edges: 100000 }
    Returns: { edges: [{source, target, strength}], scores: {msg_id: float}, count, query }
    """
    result = compute_proximity_edges(body.query_text, body.delta, body.max_edges, body.max_neighbors)
    edges = [
        {"source": str(src), "target": str(tgt), "strength": round(strength, 4)}
        for src, tgt, strength in result["edges"]
    ]
    scores = {str(k): v for k, v in result["scores"].items()}
    return {
        "edges": edges,
        "scores": scores,
        "count": len(edges),
        "query": body.query_text,
    }


# ==== Ingest Endpoint ====

@app.post("/ingest")
def trigger_ingest(
    since: str = Query(default="24h", description="Time window to ingest, e.g. '24h', '7d'"),
):
    """Trigger re-ingestion of Claude Code sessions from ~/.claude/.

    Runs ingest.py as a subprocess with --since flag.
    Returns: { sessions, messages, tools, error? }
    """
    ingest_script = FilePath(__file__).parent.parent / "ingest.py"
    if not ingest_script.exists():
        return {"error": f"ingest.py not found at {ingest_script}"}

    try:
        result = subprocess.run(
            ["python3", str(ingest_script), "--since", since],
            capture_output=True,
            text=True,
            timeout=300,
        )

        # Parse the "Done: N sessions, N messages, N tool usages" line from output
        output = result.stdout
        stats = {"sessions": 0, "messages": 0, "tools": 0, "output": output}

        for line in output.splitlines():
            if line.startswith("Done:"):
                import re as _re
                nums = _re.findall(r"(\d+)", line)
                if len(nums) >= 3:
                    stats["sessions"] = int(nums[0])
                    stats["messages"] = int(nums[1])
                    stats["tools"] = int(nums[2])

        if result.returncode != 0:
            stats["error"] = result.stderr or f"Exit code {result.returncode}"

        return stats

    except subprocess.TimeoutExpired:
        return {"error": "Ingest timed out after 300s"}
    except Exception as e:
        return {"error": str(e)}


# ==== Mail Network Endpoints ====

@app.get("/mail/network")
def mail_network():
    """Get mail network graph for agent communication visualization.

    Returns a force-directed graph representation:
    - nodes: agents (polecats, witness, mayor, etc.)
    - edges: message counts between agents (directed)
    - stats: summary statistics

    Used by the mini network graph widget in the sidebar.
    """
    return get_mail_network()


# ==== Health Auto Export Endpoints ====
# Commented out - health_ingest module not present

# @app.post("/health/ingest")
# async def health_ingest(payload: dict):
#     """Ingest health data from Health Auto Export iOS app.
#
#     Accepts webhook payloads from Health Auto Export and stores them
#     in the health schema (sleep_analysis, metrics tables).
#
#     Expected payload format:
#     {
#         "data": {
#             "metrics": [
#                 {"name": "sleep_analysis", "data": [...]},
#                 {"name": "heart_rate", "units": "bpm", "data": [...]}
#             ],
#             "workouts": [...]
#         }
#     }
#
#     Returns: { status, ingest_id, sleep_records, metric_records }
#     """
#     try:
#         result = health_ingest_payload(payload)
#         return result
#     except Exception as e:
#         return {"status": "error", "message": str(e)}
#
#
# @app.get("/health/sleep")
# def health_sleep(days: int = 7):
#     """Get recent sleep data.
#
#     Returns sleep records from the last N days.
#     """
#     records = get_recent_sleep(days)
#     # Convert datetimes to ISO strings
#     for r in records:
#         for k, v in r.items():
#             if hasattr(v, 'isoformat'):
#                 r[k] = v.isoformat()
#     return {"records": records, "count": len(records)}
#
#
# @app.get("/health/stats")
# def health_stats():
#     """Get health data statistics.
#
#     Returns counts of ingests, sleep records, and metrics.
#     """
#     return get_health_stats()


if __name__ == "__main__":
    # Bind to all interfaces for network access
    uvicorn.run(app, host="0.0.0.0", port=10800)
