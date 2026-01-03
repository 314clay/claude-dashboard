"""FastAPI wrapper for dashboard queries.

Provides REST endpoints for the Rust desktop app to fetch graph data.
"""
import sys
from pathlib import Path

# Add dashboard to path for imports
sys.path.insert(0, str(Path(__file__).parent.parent.parent / "dashboard"))

from fastapi import FastAPI, Query
from fastapi.middleware.cors import CORSMiddleware
from typing import Optional
import uvicorn

from components.queries import (
    get_graph_data,
    get_sessions,
    get_overview_metrics,
    get_session_messages,
    get_tool_usage,
    get_project_session_graph_data,
)
from components.summarizer import generate_partial_summary
from components.importance_scorer import (
    backfill_importance_scores,
    get_importance_stats,
    score_session,
)

from project_detection import (
    get_project_summary,
    detect_project_for_session,
    backfill_detected_projects,
)

app = FastAPI(
    title="Dashboard API",
    description="REST API for Claude Activity Dashboard",
    version="0.1.0",
)

# Allow requests from the Rust app
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["GET", "POST"],
    allow_headers=["*"],
)


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
    batch_size: int = Query(default=25, description="Messages per LLM call"),
):
    """Backfill importance scores for unscored messages.

    Calls Gemini to score messages on importance (0.0-1.0).
    This may take 2-5 seconds per session.
    """
    return backfill_importance_scores(max_sessions, batch_size)


@app.post("/importance/session/{session_id}")
def importance_score_session(
    session_id: str,
    batch_size: int = Query(default=25, description="Messages per LLM call"),
):
    """Score messages in a specific session.

    Calls Gemini to score unscored messages in the session.
    """
    return score_session(session_id, batch_size)


if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=8000)
