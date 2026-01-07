"""Importance scoring package.

Uses session context (summaries) to score individual message importance.
Two-phase approach: expensive summary generation, then cheap per-message scoring.
"""
from .context import SessionContext, SessionContextManager
from .scorer import ImportanceScorer
from .backfill import backfill_importance_scores, get_importance_stats

__all__ = [
    'SessionContext',
    'SessionContextManager',
    'ImportanceScorer',
    'backfill_importance_scores',
    'get_importance_stats',
]
