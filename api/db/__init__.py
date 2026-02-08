"""Database module for dashboard-native API.

Provides query functions, summarization, importance scoring, and semantic filters.
"""
from .queries import (
    get_connection,
    get_graph_data,
    get_sessions,
    get_overview_metrics,
    get_session_messages,
    get_session_summary,
    get_tool_usage,
    get_project_session_graph_data,
)
from .summarizer import generate_partial_summary, get_or_create_summary
from .semantic_filters import (
    get_all_filters,
    create_filter,
    delete_filter,
    get_filter_status,
    get_message_filter_matches,
)
from .semantic_filter_scorer import (
    categorize_messages,
    get_filter_stats,
)
from .embeddings import (
    get_embedding_stats,
    generate_embeddings,
    search_by_query,
)

__all__ = [
    'get_connection',
    'get_graph_data',
    'get_sessions',
    'get_overview_metrics',
    'get_session_messages',
    'get_session_summary',
    'get_tool_usage',
    'get_project_session_graph_data',
    'generate_partial_summary',
    'get_or_create_summary',
    'get_all_filters',
    'create_filter',
    'delete_filter',
    'get_filter_status',
    'get_message_filter_matches',
    'categorize_messages',
    'get_filter_stats',
    'get_embedding_stats',
    'generate_embeddings',
    'search_by_query',
]
