"""Tests for api/db/filter_engine.py using in-memory SQLite."""
import sqlite3
from datetime import datetime, timedelta, timezone

import pytest

from db.filter_engine import compute_visible_set, _bfs_expand, _build_adjacency_list


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _hours_ago(h: float) -> str:
    return (datetime.now(timezone.utc) - timedelta(hours=h)).isoformat()


def _create_schema(conn: sqlite3.Connection):
    """Create the minimal schema needed for filter engine tests."""
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS sessions (
            session_id  TEXT PRIMARY KEY,
            cwd         TEXT NOT NULL,
            start_time  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS messages (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id   TEXT NOT NULL,
            role         TEXT NOT NULL,
            content      TEXT NOT NULL,
            sequence_num INTEGER NOT NULL,
            timestamp    TEXT DEFAULT (datetime('now')),
            FOREIGN KEY (session_id) REFERENCES sessions (session_id),
            UNIQUE (session_id, sequence_num)
        );

        CREATE TABLE IF NOT EXISTS semantic_filters (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            query_text  TEXT NOT NULL,
            filter_type TEXT NOT NULL DEFAULT 'semantic',
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            is_active   INTEGER NOT NULL DEFAULT 1,
            UNIQUE (name)
        );

        CREATE TABLE IF NOT EXISTS semantic_filter_results (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            filter_id   INTEGER NOT NULL,
            message_id  INTEGER NOT NULL,
            matches     INTEGER NOT NULL,
            confidence  REAL,
            scored_at   TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (filter_id) REFERENCES semantic_filters (id),
            FOREIGN KEY (message_id) REFERENCES messages (id),
            UNIQUE (filter_id, message_id)
        );
    """)


def _make_conn() -> sqlite3.Connection:
    """Create an in-memory SQLite connection with Row factory and schema."""
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    _create_schema(conn)
    return conn


def _seed_graph(conn: sqlite3.Connection):
    """Seed a test graph with 10 messages across 2 sessions.

    Session A (sess-a): messages 1-5 (seq 1-5)
    Session B (sess-b): messages 6-10 (seq 1-5)

    Structural edges (consecutive in same session):
      Session A: 1-2, 2-3, 3-4, 4-5
      Session B: 6-7, 7-8, 8-9, 9-10

    Graph topology for BFS:
      1 -- 2 -- 3 -- 4 -- 5
      6 -- 7 -- 8 -- 9 -- 10
    """
    ts = _hours_ago(1)  # All messages within the last hour

    conn.execute("INSERT INTO sessions VALUES ('sess-a', '/proj/a', ?)", (ts,))
    conn.execute("INSERT INTO sessions VALUES ('sess-b', '/proj/b', ?)", (ts,))

    for i in range(1, 6):
        role = "user" if i % 2 == 1 else "assistant"
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, sequence_num, timestamp) VALUES (?, ?, ?, ?, ?, ?)",
            (i, "sess-a", role, f"message {i}", i, ts),
        )

    for i in range(6, 11):
        role = "user" if i % 2 == 0 else "assistant"
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, sequence_num, timestamp) VALUES (?, ?, ?, ?, ?, ?)",
            (i, "sess-b", role, f"message {i}", i - 5, ts),
        )

    conn.commit()


def _seed_filters(conn: sqlite3.Connection):
    """Create two test filters.

    Filter 1 ("code-review"): matches messages 2, 4, 7
    Filter 2 ("debugging"):   matches messages 3, 8, 9
    """
    conn.execute(
        "INSERT INTO semantic_filters (id, name, query_text) VALUES (1, 'code-review', 'code review discussions')"
    )
    conn.execute(
        "INSERT INTO semantic_filters (id, name, query_text) VALUES (2, 'debugging', 'debugging sessions')"
    )

    # Filter 1 matches: 2, 4, 7
    for mid in [2, 4, 7]:
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches, confidence) VALUES (1, ?, 1, 0.9)",
            (mid,),
        )

    # Filter 2 matches: 3, 8, 9
    for mid in [3, 8, 9]:
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches, confidence) VALUES (2, ?, 1, 0.9)",
            (mid,),
        )

    # Also add some non-matches to make sure they're excluded
    for mid in [1, 5, 6]:
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches, confidence) VALUES (1, ?, 0, 0.1)",
            (mid,),
        )

    conn.commit()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestAllFiltersOff:
    """When all filters are off, no filtering should be applied."""

    def test_returns_none_visible_ids(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "off", 2: "off"},
            hours=24,
            conn=conn,
        )

        assert result["visible_message_ids"] is None
        assert result["total_nodes"] == 10
        assert result["visible_count"] == 10

    def test_empty_filter_modes(self):
        conn = _make_conn()
        _seed_graph(conn)

        result = compute_visible_set(
            filter_modes={},
            hours=24,
            conn=conn,
        )

        assert result["visible_message_ids"] is None
        assert result["total_nodes"] == 10
        assert result["visible_count"] == 10


class TestSingleInclude:
    """Single include filter returns only matching nodes."""

    def test_include_filter_1(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "include"},
            hours=24,
            conn=conn,
        )

        # Filter 1 matches: 2, 4, 7
        assert set(result["visible_message_ids"]) == {2, 4, 7}
        assert result["visible_count"] == 3
        assert result["total_nodes"] == 10

    def test_include_filter_2(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={2: "include"},
            hours=24,
            conn=conn,
        )

        # Filter 2 matches: 3, 8, 9
        assert set(result["visible_message_ids"]) == {3, 8, 9}
        assert result["visible_count"] == 3


class TestSingleExclude:
    """Single exclude filter removes matching nodes, keeps everything else."""

    def test_exclude_filter_1(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "exclude"},
            hours=24,
            conn=conn,
        )

        # Exclude filter 1 matches (2, 4, 7) from all (1-10)
        expected = {1, 3, 5, 6, 8, 9, 10}
        assert set(result["visible_message_ids"]) == expected
        assert result["visible_count"] == 7

    def test_exclude_filter_2(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={2: "exclude"},
            hours=24,
            conn=conn,
        )

        # Exclude filter 2 matches (3, 8, 9)
        expected = {1, 2, 4, 5, 6, 7, 10}
        assert set(result["visible_message_ids"]) == expected
        assert result["visible_count"] == 7


class TestIncludePlusExclude:
    """Include + Exclude combination: include narrows, exclude removes from that."""

    def test_include_1_exclude_2(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "include", 2: "exclude"},
            hours=24,
            conn=conn,
        )

        # Include filter 1: {2, 4, 7}
        # Exclude filter 2: removes {3, 8, 9}
        # Result: {2, 4, 7} - {3, 8, 9} = {2, 4, 7}
        assert set(result["visible_message_ids"]) == {2, 4, 7}

    def test_include_2_exclude_1(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={2: "include", 1: "exclude"},
            hours=24,
            conn=conn,
        )

        # Include filter 2: {3, 8, 9}
        # Exclude filter 1: removes {2, 4, 7}
        # Result: {3, 8, 9} - {2, 4, 7} = {3, 8, 9}
        assert set(result["visible_message_ids"]) == {3, 8, 9}

    def test_overlapping_include_exclude(self):
        """When include and exclude match the same node, exclude wins."""
        conn = _make_conn()
        _seed_graph(conn)

        # Create a filter that matches msg 2 and 3
        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (10, 'f10', 'q')"
        )
        # Create a filter that matches msg 2 and 5
        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (11, 'f11', 'q')"
        )
        for mid in [2, 3]:
            conn.execute(
                "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (10, ?, 1)",
                (mid,),
            )
        for mid in [2, 5]:
            conn.execute(
                "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (11, ?, 1)",
                (mid,),
            )
        conn.commit()

        result = compute_visible_set(
            filter_modes={10: "include", 11: "exclude"},
            hours=24,
            conn=conn,
        )

        # Include: {2, 3}, Exclude removes: {2, 5}
        # Result: {3}
        assert set(result["visible_message_ids"]) == {3}


class TestIncludePlus1:
    """Include+1 expands one hop along structural edges."""

    def test_expand_from_message_3(self):
        """Message 3 in session A: neighbors are 2 and 4."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (20, 'f20', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (20, 3, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={20: "include_plus_1"},
            hours=24,
            conn=conn,
        )

        # Seed: {3}, +1 hop: {2, 4}
        assert set(result["visible_message_ids"]) == {2, 3, 4}

    def test_expand_from_edge_node(self):
        """Message 1 in session A: only neighbor is 2."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (21, 'f21', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (21, 1, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={21: "include_plus_1"},
            hours=24,
            conn=conn,
        )

        # Seed: {1}, +1 hop: {2}
        assert set(result["visible_message_ids"]) == {1, 2}

    def test_no_cross_session_expansion(self):
        """Expansion does not cross session boundaries (5 and 6 are in different sessions)."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (22, 'f22', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (22, 5, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={22: "include_plus_1"},
            hours=24,
            conn=conn,
        )

        # Seed: {5}, +1 hop: {4} (no edge to 6, different session)
        assert set(result["visible_message_ids"]) == {4, 5}


class TestIncludePlus2:
    """Include+2 expands two hops along structural edges."""

    def test_expand_from_message_3(self):
        """Message 3: +2 hops reaches 1,2,3,4,5."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (30, 'f30', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (30, 3, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={30: "include_plus_2"},
            hours=24,
            conn=conn,
        )

        # Seed: {3}, +1: {2,4}, +2: {1,5}
        assert set(result["visible_message_ids"]) == {1, 2, 3, 4, 5}

    def test_expand_from_message_8(self):
        """Message 8 in session B: +2 hops reaches 6,7,8,9,10."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (31, 'f31', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (31, 8, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={31: "include_plus_2"},
            hours=24,
            conn=conn,
        )

        # Seed: {8}, +1: {7,9}, +2: {6,10}
        assert set(result["visible_message_ids"]) == {6, 7, 8, 9, 10}

    def test_expand_from_edge_node(self):
        """Message 1: +2 hops reaches 1,2,3."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (32, 'f32', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (32, 1, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={32: "include_plus_2"},
            hours=24,
            conn=conn,
        )

        # Seed: {1}, +1: {2}, +2: {3}
        assert set(result["visible_message_ids"]) == {1, 2, 3}


class TestMultipleIncludes:
    """Multiple includes use OR/union semantics."""

    def test_two_includes_union(self):
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "include", 2: "include"},
            hours=24,
            conn=conn,
        )

        # Filter 1: {2, 4, 7}, Filter 2: {3, 8, 9}
        # Union: {2, 3, 4, 7, 8, 9}
        assert set(result["visible_message_ids"]) == {2, 3, 4, 7, 8, 9}
        assert result["visible_count"] == 6

    def test_include_plus_include_plus_1(self):
        """One filter as include, another as include_plus_1 — both contribute to union."""
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "include", 2: "include_plus_1"},
            hours=24,
            conn=conn,
        )

        # Filter 1 include: {2, 4, 7}
        # Filter 2 include+1: seeds {3, 8, 9}, +1 -> {2,4, 7,10} => {2,3,4,7,8,9,10}
        # Union: {2,3,4,7} | {2,3,4,7,8,9,10} = {2,3,4,7,8,9,10}
        assert set(result["visible_message_ids"]) == {2, 3, 4, 7, 8, 9, 10}


class TestEmptyFilterResults:
    """Filter exists but has no matches."""

    def test_include_with_no_matches(self):
        conn = _make_conn()
        _seed_graph(conn)

        # Create filter with zero matches
        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (40, 'empty', 'q')"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={40: "include"},
            hours=24,
            conn=conn,
        )

        # Include with no matches -> empty visible set
        assert result["visible_message_ids"] == []
        assert result["visible_count"] == 0

    def test_exclude_with_no_matches(self):
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (41, 'empty2', 'q')"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={41: "exclude"},
            hours=24,
            conn=conn,
        )

        # Exclude with no matches -> everything visible
        assert set(result["visible_message_ids"]) == set(range(1, 11))
        assert result["visible_count"] == 10


class TestAllNodesFilteredOut:
    """When includes are active but nothing matches visible data."""

    def test_all_filtered_out(self):
        conn = _make_conn()
        _seed_graph(conn)

        # Create filter that only matches non-existent messages
        conn.execute(
            "INSERT INTO semantic_filters (id, name, query_text) VALUES (50, 'ghost', 'q')"
        )
        conn.execute(
            "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (50, 999, 1)"
        )
        conn.commit()

        result = compute_visible_set(
            filter_modes={50: "include"},
            hours=24,
            conn=conn,
        )

        # msg 999 doesn't exist in the time range, so intersection with all_ids is empty
        assert result["visible_message_ids"] == []
        assert result["visible_count"] == 0

    def test_include_then_exclude_everything(self):
        """Include a few, then exclude all of them."""
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute("INSERT INTO semantic_filters (id, name, query_text) VALUES (51, 'f51', 'q')")
        conn.execute("INSERT INTO semantic_filters (id, name, query_text) VALUES (52, 'f52', 'q')")

        # f51 matches msg 2, 3
        for mid in [2, 3]:
            conn.execute(
                "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (51, ?, 1)",
                (mid,),
            )
        # f52 also matches msg 2, 3
        for mid in [2, 3]:
            conn.execute(
                "INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (52, ?, 1)",
                (mid,),
            )
        conn.commit()

        result = compute_visible_set(
            filter_modes={51: "include", 52: "exclude"},
            hours=24,
            conn=conn,
        )

        # Include: {2,3}, Exclude removes: {2,3} -> empty
        assert result["visible_message_ids"] == []
        assert result["visible_count"] == 0


class TestTimeRangeScoping:
    """Messages outside the time range are not included."""

    def test_old_messages_excluded(self):
        conn = _make_conn()
        _create_schema(conn)

        recent = _hours_ago(1)
        old = _hours_ago(100)

        conn.execute("INSERT INTO sessions VALUES ('s1', '/proj', ?)", (old,))

        # Recent message
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, sequence_num, timestamp) VALUES (1, 's1', 'user', 'recent', 1, ?)",
            (recent,),
        )
        # Old message
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, sequence_num, timestamp) VALUES (2, 's1', 'assistant', 'old', 2, ?)",
            (old,),
        )

        conn.execute("INSERT INTO semantic_filters (id, name, query_text) VALUES (60, 'f60', 'q')")
        # Both match the filter
        conn.execute("INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (60, 1, 1)")
        conn.execute("INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (60, 2, 1)")
        conn.commit()

        result = compute_visible_set(
            filter_modes={60: "include"},
            hours=24,
            conn=conn,
        )

        # Only message 1 is within 24h
        assert result["total_nodes"] == 1
        assert set(result["visible_message_ids"]) == {1}


class TestBFSExpand:
    """Unit tests for _bfs_expand."""

    def test_depth_0(self):
        adj = {1: [2], 2: [1, 3], 3: [2]}
        assert _bfs_expand({2}, 0, adj) == {2}

    def test_depth_1(self):
        adj = {1: [2], 2: [1, 3], 3: [2]}
        assert _bfs_expand({2}, 1, adj) == {1, 2, 3}

    def test_depth_2_linear(self):
        adj = {1: [2], 2: [1, 3], 3: [2, 4], 4: [3]}
        assert _bfs_expand({1}, 2, adj) == {1, 2, 3}

    def test_disconnected_node(self):
        adj = {1: [2], 2: [1]}
        # Node 5 has no neighbors
        assert _bfs_expand({5}, 1, adj) == {5}

    def test_multiple_seeds(self):
        adj = {1: [2], 2: [1, 3], 3: [2], 10: [11], 11: [10]}
        assert _bfs_expand({1, 10}, 1, adj) == {1, 2, 10, 11}


class TestExpandWithExclude:
    """Include+N expansion followed by exclude."""

    def test_plus1_then_exclude(self):
        conn = _make_conn()
        _seed_graph(conn)

        conn.execute("INSERT INTO semantic_filters (id, name, query_text) VALUES (70, 'f70', 'q')")
        conn.execute("INSERT INTO semantic_filters (id, name, query_text) VALUES (71, 'f71', 'q')")

        # f70 matches msg 3 (include_plus_1 -> {2,3,4})
        conn.execute("INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (70, 3, 1)")
        # f71 matches msg 2 (exclude)
        conn.execute("INSERT INTO semantic_filter_results (filter_id, message_id, matches) VALUES (71, 2, 1)")
        conn.commit()

        result = compute_visible_set(
            filter_modes={70: "include_plus_1", 71: "exclude"},
            hours=24,
            conn=conn,
        )

        # Include+1 from {3}: {2,3,4}, then exclude {2} -> {3,4}
        assert set(result["visible_message_ids"]) == {3, 4}


class TestMixedModes:
    """Complex combinations of filter modes."""

    def test_include_plus2_and_include_and_exclude(self):
        """Two includes (one with expansion) + one exclude."""
        conn = _make_conn()
        _seed_graph(conn)
        _seed_filters(conn)

        result = compute_visible_set(
            filter_modes={1: "include_plus_2", 2: "exclude"},
            hours=24,
            conn=conn,
        )

        # Filter 1 include+2: seeds {2,4,7}
        #   From 2: +1={1,3}, +2={4}  -> {1,2,3,4}
        #   From 4: +1={3,5}, +2={2}  -> {2,3,4,5}
        #   From 7: +1={6,8}, +2={9}  -> {6,7,8,9}   (note: 7 is at position 2 in session B, neighbors 6,8)
        #   Union: {1,2,3,4,5,6,7,8,9}
        # Filter 2 exclude: removes {3, 8, 9}
        # Result: {1,2,4,5,6,7}
        assert set(result["visible_message_ids"]) == {1, 2, 4, 5, 6, 7}
