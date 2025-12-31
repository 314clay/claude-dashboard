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
    allow_methods=["GET"],
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


@app.get("/tools")
def tools(
    hours: float = Query(default=24, description="Hours to look back"),
):
    """Get tool usage statistics."""
    df = get_tool_usage(hours)
    if df.empty:
        return {"tools": []}

    return {"tools": df.to_dict(orient="records")}


if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=8000)
