"""Compute visible message set based on semantic filter configurations.

Moves ALL filter visibility logic from Rust to the Python backend.
The Rust app sends filter_modes and hours, and this module returns the
set of message IDs that should be visible.
"""
import sqlite3
from collections import defaultdict
from datetime import datetime, timedelta, timezone

from .queries import get_connection, DB_PATH


# SQLite variable limit (~999), batch IN clauses to this size
_BATCH_SIZE = 900


def _batched_query(cur, query_template: str, ids: list[int], extra_params: tuple = ()) -> list:
    """Execute a query with an IN clause, batching to avoid SQLite limits.

    query_template must contain {placeholders} where the IN list goes.
    """
    rows = []
    for i in range(0, len(ids), _BATCH_SIZE):
        batch = ids[i:i + _BATCH_SIZE]
        placeholders = ",".join("?" * len(batch))
        sql = query_template.format(placeholders=placeholders)
        cur.execute(sql, (*extra_params, *batch))
        rows.extend(cur.fetchall())
    return rows


def _build_adjacency_list(cur, message_ids: set[int], hours: float) -> dict[int, list[int]]:
    """Build undirected adjacency list from structural (sequential) edges.

    Structural edges are consecutive messages within the same session,
    ordered by (session_id, sequence_num). This mirrors how get_graph_data
    in queries.py builds edges.
    """
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    cur.execute("""
        SELECT m.id, m.session_id, m.sequence_num
        FROM messages m
        JOIN sessions s ON m.session_id = s.session_id
        WHERE m.timestamp >= ?
        ORDER BY m.session_id, m.sequence_num
    """, (since,))

    rows = cur.fetchall()

    adj: dict[int, list[int]] = defaultdict(list)
    prev_id = None
    prev_session = None

    for row in rows:
        msg_id = row["id"]
        session_id = row["session_id"]

        if session_id == prev_session and prev_id is not None:
            # Undirected edge between consecutive messages in same session
            adj[prev_id].append(msg_id)
            adj[msg_id].append(prev_id)

        prev_id = msg_id
        prev_session = session_id

    return dict(adj)


def _bfs_expand(seeds: set[int], depth: int, adj: dict[int, list[int]]) -> set[int]:
    """BFS expansion from seed nodes to the given depth."""
    visited = set(seeds)
    frontier = set(seeds)

    for _ in range(depth):
        next_frontier = set()
        for node_id in frontier:
            for neighbor in adj.get(node_id, []):
                if neighbor not in visited:
                    visited.add(neighbor)
                    next_frontier.add(neighbor)
        frontier = next_frontier

    return visited


def compute_visible_set(
    filter_modes: dict[int, str],
    hours: float,
    conn: sqlite3.Connection | None = None,
) -> dict:
    """Compute the set of visible message IDs based on semantic filter modes.

    Args:
        filter_modes: {filter_id: mode_string} where mode_string is one of
                      "off", "exclude", "include", "include_plus_1", "include_plus_2"
        hours: Time range (hours back from now) to scope message data.
        conn: Optional database connection (for testing with in-memory DBs).

    Returns:
        dict with:
            visible_message_ids: list[int] | None  (None means no filtering)
            total_nodes: int
            visible_count: int
    """
    owns_conn = conn is None
    if owns_conn:
        conn = get_connection()

    cur = conn.cursor()

    try:
        # Get all message IDs in time range
        since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()
        cur.execute("""
            SELECT m.id
            FROM messages m
            JOIN sessions s ON m.session_id = s.session_id
            WHERE m.timestamp >= ?
        """, (since,))
        all_ids = {row["id"] for row in cur.fetchall()}
        total_nodes = len(all_ids)

        # Filter out "off" modes
        active_modes = {
            fid: mode for fid, mode in filter_modes.items()
            if mode != "off"
        }

        # If no active filters, return None (no filtering applied)
        if not active_modes:
            return {
                "visible_message_ids": None,
                "total_nodes": total_nodes,
                "visible_count": total_nodes,
            }

        # Load filter match results for all active filter IDs
        active_filter_ids = list(active_modes.keys())
        match_rows = _batched_query(
            cur,
            """
            SELECT filter_id, message_id
            FROM semantic_filter_results
            WHERE filter_id IN ({placeholders})
              AND matches = 1
            """,
            active_filter_ids,
        )

        # Build filter_id -> set of matching message_ids
        filter_matches: dict[int, set[int]] = defaultdict(set)
        for row in match_rows:
            filter_matches[row["filter_id"]].add(row["message_id"])

        # Check if any include-type filters exist
        include_modes = {"include", "include_plus_1", "include_plus_2"}
        has_includes = any(m in include_modes for m in active_modes.values())

        # Build adjacency list only if expansion is needed
        needs_expansion = any(
            m in ("include_plus_1", "include_plus_2")
            for m in active_modes.values()
        )
        adj = _build_adjacency_list(cur, all_ids, hours) if needs_expansion else {}

        # Compute include union (OR semantics across all include filters)
        include_union: set[int] = set()

        for fid, mode in active_modes.items():
            matching = filter_matches.get(fid, set()) & all_ids

            if mode == "include":
                include_union |= matching
            elif mode == "include_plus_1":
                expanded = _bfs_expand(matching, 1, adj)
                include_union |= (expanded & all_ids)
            elif mode == "include_plus_2":
                expanded = _bfs_expand(matching, 2, adj)
                include_union |= (expanded & all_ids)

        # Start with include union (or all nodes if no includes)
        visible = include_union if has_includes else set(all_ids)

        # Apply exclude filters (remove matching nodes)
        for fid, mode in active_modes.items():
            if mode == "exclude":
                exclude_matches = filter_matches.get(fid, set())
                visible -= exclude_matches

        return {
            "visible_message_ids": sorted(visible),
            "total_nodes": total_nodes,
            "visible_count": len(visible),
        }

    finally:
        cur.close()
        if owns_conn:
            conn.close()
