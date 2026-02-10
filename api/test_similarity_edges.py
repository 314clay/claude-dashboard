"""Tests for compute_proximity_edges: stability and quality properties.

Tests use synthetic normalized embedding matrices so they run without
a database or API keys. We mock search_by_query to return controlled scores.
"""

import numpy as np
import pytest
from unittest.mock import patch


from db.embeddings import compute_proximity_edges


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _edge_set(edges: list[tuple[int, int, float]]) -> set[tuple[int, int]]:
    """Extract (src, tgt) pairs, ignoring strengths."""
    return {(s, t) for s, t, _ in edges}


def _make_scores(n: int, score_fn=None) -> dict[int, float]:
    """Build a scores dict mapping message_id -> score.

    score_fn(i, n) returns score for message i (0-indexed).
    Default: linearly spaced 0.0 to 1.0.
    """
    if score_fn is None:
        score_fn = lambda i, n: i / max(n - 1, 1)
    return {i + 1: score_fn(i, n) for i in range(n)}


# ---------------------------------------------------------------------------
# 1. Basic edge creation
# ---------------------------------------------------------------------------

class TestBasicEdges:
    """Proximity edges link nodes within delta of each other."""

    def test_all_same_score_fully_connected(self):
        """When all nodes have the same score, all pairs within delta."""
        scores = {1: 0.5, 2: 0.5, 3: 0.5, 4: 0.5}
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)

        # 4 nodes all at same score → C(4,2) = 6 edges
        assert result["count"] == 6
        assert len(result["edges"]) == 6

    def test_two_clusters_no_cross_edges(self):
        """Two clusters separated by more than delta produce no cross-edges."""
        scores = {1: 0.0, 2: 0.01, 3: 0.02,   # low cluster
                  4: 0.5, 5: 0.51, 6: 0.52}     # high cluster
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.05)

        edges = result["edges"]
        # All edges should be within-cluster
        for src, tgt, _ in edges:
            low = {1, 2, 3}
            high = {4, 5, 6}
            same_cluster = (src in low and tgt in low) or (src in high and tgt in high)
            assert same_cluster, f"Cross-cluster edge ({src}, {tgt})"

    def test_empty_scores_returns_empty(self):
        """No scores → no edges."""
        with patch("db.embeddings.search_by_query", return_value={}):
            result = compute_proximity_edges("test")
        assert result["count"] == 0
        assert result["edges"] == []
        assert result["scores"] == {}

    def test_single_node_no_edges(self):
        """One node can't form edges."""
        with patch("db.embeddings.search_by_query", return_value={1: 0.5}):
            result = compute_proximity_edges("test")
        assert result["count"] == 0


# ---------------------------------------------------------------------------
# 2. Delta monotonicity
# ---------------------------------------------------------------------------

class TestDeltaMonotonicity:
    """Increasing delta must produce a superset of edges."""

    def test_larger_delta_is_superset(self):
        scores = _make_scores(20)
        deltas = [0.02, 0.05, 0.1, 0.2, 0.5]

        with patch("db.embeddings.search_by_query", return_value=scores):
            prev_edges = None
            prev_d = None
            for d in deltas:
                edges = _edge_set(
                    compute_proximity_edges("test", delta=d, max_edges=1_000_000)["edges"]
                )
                if prev_edges is not None:
                    assert prev_edges <= edges, (
                        f"delta {d} lost edges from delta {prev_d}: "
                        f"{prev_edges - edges}"
                    )
                prev_edges = edges
                prev_d = d


# ---------------------------------------------------------------------------
# 3. Strength properties
# ---------------------------------------------------------------------------

class TestStrength:
    """Edge strength = 1 - |score_diff| / delta."""

    def test_same_score_has_strength_1(self):
        scores = {1: 0.5, 2: 0.5}
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)
        assert len(result["edges"]) == 1
        _, _, strength = result["edges"][0]
        assert strength == pytest.approx(1.0)

    def test_strength_decreases_with_distance(self):
        scores = {1: 0.5, 2: 0.55, 3: 0.59}
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)

        edge_dict = {(min(s, t), max(s, t)): st for s, t, st in result["edges"]}

        # 1-2 closer than 1-3
        if (1, 2) in edge_dict and (1, 3) in edge_dict:
            assert edge_dict[(1, 2)] > edge_dict[(1, 3)]

    def test_strength_is_zero_at_boundary(self):
        """Nodes exactly delta apart should have strength ≈ 0."""
        scores = {1: 0.5, 2: 0.6}  # diff = 0.1 = delta
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)

        # Should still produce an edge (diff <= delta), but strength ≈ 0
        assert len(result["edges"]) == 1
        _, _, strength = result["edges"][0]
        assert strength == pytest.approx(0.0, abs=1e-6)

    def test_all_strengths_in_range(self):
        """All edge strengths must be in [0.0, 1.0]."""
        scores = _make_scores(50)
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)

        for _, _, strength in result["edges"]:
            assert 0.0 <= strength <= 1.0 + 1e-6, f"Strength {strength} out of range"


# ---------------------------------------------------------------------------
# 4. Max edges cap
# ---------------------------------------------------------------------------

class TestMaxEdges:
    """max_edges caps the total returned edges."""

    def test_cap_limits_count(self):
        scores = _make_scores(100)  # many nodes → many edges with delta=0.5
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.5, max_edges=10)
        assert result["count"] <= 10
        assert len(result["edges"]) <= 10

    def test_uncapped_returns_more(self):
        scores = _make_scores(50)
        with patch("db.embeddings.search_by_query", return_value=scores):
            capped = compute_proximity_edges("test", delta=0.5, max_edges=5)
            uncapped = compute_proximity_edges("test", delta=0.5, max_edges=1_000_000)
        assert uncapped["count"] >= capped["count"]


# ---------------------------------------------------------------------------
# 5. Scores passthrough
# ---------------------------------------------------------------------------

class TestScoresPassthrough:
    """The returned scores dict should match what search_by_query provides."""

    def test_scores_returned_unchanged(self):
        scores = {1: 0.1, 2: 0.5, 3: 0.9}
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)
        assert result["scores"] == scores


# ---------------------------------------------------------------------------
# 6. Symmetry (undirected)
# ---------------------------------------------------------------------------

class TestSymmetry:
    """Each pair appears only once (no duplicates)."""

    def test_no_duplicate_pairs(self):
        scores = _make_scores(20)
        with patch("db.embeddings.search_by_query", return_value=scores):
            result = compute_proximity_edges("test", delta=0.1)

        seen = set()
        for src, tgt, _ in result["edges"]:
            pair = (min(src, tgt), max(src, tgt))
            assert pair not in seen, f"Duplicate edge: {pair}"
            seen.add(pair)
