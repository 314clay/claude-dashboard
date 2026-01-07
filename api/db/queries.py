"""Database query functions for the dashboard."""
import os
import re
import psycopg2
from psycopg2.extras import RealDictCursor
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
import pandas as pd

# Support both local development and container networking
DB_CONFIG = {
    "host": os.environ.get("DB_HOST", "localhost"),
    "port": int(os.environ.get("DB_PORT", "5433")),
    "user": os.environ.get("DB_USER", "clayarnold"),
    "database": os.environ.get("DB_NAME", "connectingservices"),
}

# Home directory for path normalization (configurable via env var)
HOME_DIR = os.environ.get("USER_HOME", str(Path.home()))


def _normalize_path(path: str) -> str:
    """Normalize a path by replacing home directory with ~."""
    if path and HOME_DIR and path.startswith(HOME_DIR):
        return path.replace(HOME_DIR, "~", 1)
    return path


def get_connection():
    """Get database connection."""
    return psycopg2.connect(**DB_CONFIG, cursor_factory=RealDictCursor)


def _query_to_df(cur, query, params=None) -> pd.DataFrame:
    """Execute query and return DataFrame."""
    cur.execute(query, params)
    rows = cur.fetchall()
    if not rows:
        return pd.DataFrame()
    return pd.DataFrame([dict(row) for row in rows])


def get_overview_metrics(hours: float = 24) -> dict:
    """Get high-level metrics for the overview."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    cur.execute("""
        SELECT
            (SELECT COUNT(*) FROM claude_sessions.sessions WHERE start_time >= %s) as session_count,
            (SELECT COUNT(*) FROM claude_sessions.messages WHERE timestamp >= %s) as message_count,
            (SELECT COUNT(*) FROM claude_sessions.messages WHERE timestamp >= %s AND role = 'user') as user_messages,
            (SELECT COUNT(*) FROM claude_sessions.messages WHERE timestamp >= %s AND role = 'assistant') as assistant_messages,
            (SELECT COUNT(*) FROM claude_sessions.tool_usages WHERE timestamp >= %s) as tool_count
    """, (since, since, since, since, since))

    row = cur.fetchone()
    cur.close()
    conn.close()

    return dict(row)


def get_sessions(hours: float = 24, limit: int = 50) -> pd.DataFrame:
    """Get sessions with message counts."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    query = """
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            s.end_time,
            COUNT(m.id) as total_messages,
            COUNT(*) FILTER (WHERE m.role = 'user') as user_messages,
            COUNT(*) FILTER (WHERE m.role = 'assistant') as assistant_messages,
            EXTRACT(EPOCH FROM (COALESCE(s.end_time, NOW()) - s.start_time))/60 as duration_mins
        FROM claude_sessions.sessions s
        LEFT JOIN claude_sessions.messages m ON s.session_id = m.session_id
        WHERE s.start_time >= %s
        GROUP BY s.session_id, s.cwd, s.start_time, s.end_time
        ORDER BY s.start_time DESC
        LIMIT %s
    """

    df = _query_to_df(cur, query, (since, limit))
    cur.close()
    conn.close()

    if df.empty:
        return df

    # Convert Decimal to float
    if 'duration_mins' in df.columns:
        df['duration_mins'] = df['duration_mins'].apply(lambda x: float(x) if isinstance(x, Decimal) else x)

    # Clean up paths
    df['project'] = df['cwd'].apply(_normalize_path)
    df['is_active'] = df['end_time'].isna()

    return df


def get_session_messages(session_id: str) -> pd.DataFrame:
    """Get all messages for a specific session."""
    conn = get_connection()
    cur = conn.cursor()

    query = """
        SELECT
            m.id,
            m.role,
            m.content,
            m.timestamp,
            m.sequence_num
        FROM claude_sessions.messages m
        WHERE m.session_id = %s
        ORDER BY m.sequence_num
    """

    df = _query_to_df(cur, query, (session_id,))
    cur.close()
    conn.close()
    return df


def get_session_messages_before(session_id: str, before_timestamp: str) -> pd.DataFrame:
    """Get messages for a session up to (and including) a specific timestamp.

    Args:
        session_id: The session UUID
        before_timestamp: ISO8601 timestamp - only include messages at or before this time

    Returns:
        DataFrame with columns: id, role, content, timestamp, sequence_num
    """
    conn = get_connection()
    cur = conn.cursor()

    query = """
        SELECT
            m.id,
            m.role,
            m.content,
            m.timestamp,
            m.sequence_num
        FROM claude_sessions.messages m
        WHERE m.session_id = %s
          AND m.timestamp <= %s
        ORDER BY m.sequence_num
    """

    df = _query_to_df(cur, query, (session_id, before_timestamp))
    cur.close()
    conn.close()
    return df


def get_session_summary(session_id: str) -> dict | None:
    """Get the full session summary from session_summaries table.

    Args:
        session_id: The session UUID

    Returns:
        dict with summary, user_requests, completed_work, topics, detected_project, generated_at
        or None if no summary exists
    """
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT
            summary,
            user_requests,
            completed_work,
            topics,
            detected_project,
            generated_at
        FROM claude_sessions.session_summaries
        WHERE session_id = %s
    """, (session_id,))

    row = cur.fetchone()
    cur.close()
    conn.close()

    if not row:
        return None

    result = dict(row)
    # Convert datetime to ISO string for JSON serialization
    if result.get('generated_at') and hasattr(result['generated_at'], 'isoformat'):
        result['generated_at'] = result['generated_at'].isoformat()

    return result


def get_session_tools(session_id: str) -> pd.DataFrame:
    """Get tool usages for a specific session."""
    conn = get_connection()
    cur = conn.cursor()

    query = """
        SELECT
            t.tool_name,
            t.tool_input,
            t.timestamp,
            t.sequence_num
        FROM claude_sessions.tool_usages t
        JOIN claude_sessions.messages m ON t.message_id = m.id
        WHERE m.session_id = %s
        ORDER BY t.timestamp
    """

    df = _query_to_df(cur, query, (session_id,))
    cur.close()
    conn.close()
    return df


def get_tool_usage(hours: float = 24) -> pd.DataFrame:
    """Get tool usage statistics."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    query = """
        SELECT
            CASE
                WHEN tool_name LIKE 'mcp__%%' THEN 'MCP: ' || split_part(tool_name, '__', 2)
                ELSE tool_name
            END as tool_category,
            tool_name,
            COUNT(*) as usage_count
        FROM claude_sessions.tool_usages
        WHERE timestamp >= %s
        GROUP BY tool_name
        ORDER BY usage_count DESC
    """

    df = _query_to_df(cur, query, (since,))
    cur.close()
    conn.close()
    return df


def get_activity_by_hour(hours: float = 168) -> pd.DataFrame:
    """Get activity breakdown by hour of day (default: past week)."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    query = """
        SELECT
            EXTRACT(HOUR FROM m.timestamp) as hour_of_day,
            EXTRACT(DOW FROM m.timestamp) as day_of_week,
            COUNT(*) FILTER (WHERE m.role = 'user') as user_messages,
            COUNT(*) FILTER (WHERE m.role = 'assistant') as assistant_messages,
            COUNT(*) as total_messages
        FROM claude_sessions.messages m
        WHERE m.timestamp >= %s
        GROUP BY 1, 2
        ORDER BY 2, 1
    """

    df = _query_to_df(cur, query, (since,))
    cur.close()
    conn.close()

    # Convert Decimals to int
    if not df.empty:
        for col in ['hour_of_day', 'day_of_week']:
            if col in df.columns:
                df[col] = df[col].apply(lambda x: int(x) if isinstance(x, Decimal) else x)

    return df


def get_activity_timeline(hours: float = 24) -> pd.DataFrame:
    """Get activity timeline with hourly buckets."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    query = """
        SELECT
            date_trunc('hour', m.timestamp) as hour_bucket,
            COUNT(*) FILTER (WHERE m.role = 'user') as user_messages,
            COUNT(*) FILTER (WHERE m.role = 'assistant') as assistant_messages,
            COUNT(DISTINCT m.session_id) as active_sessions
        FROM claude_sessions.messages m
        WHERE m.timestamp >= %s
        GROUP BY 1
        ORDER BY 1
    """

    df = _query_to_df(cur, query, (since,))
    cur.close()
    conn.close()
    return df


def get_projects(hours: float = 168) -> pd.DataFrame:
    """Get project/directory breakdown."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    # Use a more generic approach - extract last two path components
    query = """
        SELECT
            s.cwd as project,
            COUNT(DISTINCT s.session_id) as session_count,
            COUNT(m.id) as message_count
        FROM claude_sessions.sessions s
        LEFT JOIN claude_sessions.messages m ON s.session_id = m.session_id
        WHERE s.start_time >= %s
        GROUP BY 1
        ORDER BY message_count DESC
    """

    df = _query_to_df(cur, query, (since,))
    cur.close()
    conn.close()

    # Normalize paths in Python for flexibility
    if not df.empty:
        df['project'] = df['project'].apply(_normalize_path)

    return df


def get_all_messages(hours: float = 24, limit: int = 500) -> pd.DataFrame:
    """Get all messages with session info for data table."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    query = """
        SELECT
            m.id,
            m.session_id,
            m.role,
            m.content,
            m.timestamp,
            m.sequence_num,
            s.cwd
        FROM claude_sessions.messages m
        JOIN claude_sessions.sessions s ON m.session_id = s.session_id
        WHERE m.timestamp >= %s
        ORDER BY m.timestamp DESC
        LIMIT %s
    """

    df = _query_to_df(cur, query, (since, limit))
    cur.close()
    conn.close()

    if not df.empty:
        df['project'] = df['cwd'].apply(_normalize_path)
        df['session_short'] = df['session_id'].str[:8]

    return df


def get_recent_user_messages(hours: float = 24, limit: int = 20) -> pd.DataFrame:
    """Get recent user messages for topic overview."""
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    cur.execute("""
        SELECT
            m.content,
            m.timestamp,
            m.session_id,
            s.cwd
        FROM claude_sessions.messages m
        JOIN claude_sessions.sessions s ON m.session_id = s.session_id
        WHERE m.role = 'user' AND m.timestamp >= %s
        ORDER BY m.timestamp DESC
        LIMIT %s
    """, (since, limit))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    return pd.DataFrame([dict(row) for row in rows]) if rows else pd.DataFrame()


def get_graph_data(hours: float = 24, session_filter: str = None) -> tuple[list, list]:
    """Get nodes and links for graph visualization.

    Returns:
        tuple: (nodes, links) where
            nodes = [{ id, role, content_preview, session_id, timestamp, importance_score, semantic_filter_matches }, ...]
            links = [{ source, target, session_id }, ...]
    """
    # Import here to avoid circular dependency
    from .semantic_filters import get_message_filter_matches

    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    # Build query with optional session filter
    if session_filter:
        query = """
            SELECT
                m.id,
                m.session_id,
                m.role,
                m.content,
                m.timestamp,
                m.sequence_num,
                m.importance_score,
                m.importance_reason,
                m.token_count,
                m.input_tokens,
                m.cache_read_tokens,
                m.cache_creation_tokens,
                s.cwd
            FROM claude_sessions.messages m
            JOIN claude_sessions.sessions s ON m.session_id = s.session_id
            WHERE m.session_id = %s
            ORDER BY m.session_id, m.sequence_num
        """
        params = (session_filter,)
    else:
        query = """
            SELECT
                m.id,
                m.session_id,
                m.role,
                m.content,
                m.timestamp,
                m.sequence_num,
                m.importance_score,
                m.importance_reason,
                m.token_count,
                m.input_tokens,
                m.cache_read_tokens,
                m.cache_creation_tokens,
                s.cwd
            FROM claude_sessions.messages m
            JOIN claude_sessions.sessions s ON m.session_id = s.session_id
            WHERE m.timestamp >= %s
            ORDER BY m.session_id, m.sequence_num
        """
        params = (since,)

    cur.execute(query, params)
    rows = cur.fetchall()
    cur.close()
    conn.close()

    if not rows:
        return [], []

    # Collect all message IDs to fetch filter matches in one query
    message_ids = [row['id'] for row in rows]
    filter_matches = get_message_filter_matches(message_ids)

    nodes = []
    links = []
    prev_msg = {}  # Track previous message per session

    for row in rows:
        msg_id = str(row['id'])
        msg_id_int = row['id']
        session_id = row['session_id']
        role = row['role']
        content = row['content'] or ""

        # Create node
        nodes.append({
            'id': msg_id,
            'role': role,
            'content_preview': content[:100] + '...' if len(content) > 100 else content,
            'full_content': content,  # Full content for detail panel
            'session_id': session_id,
            'session_short': session_id[:8],
            'project': _normalize_path(row['cwd']) if row['cwd'] else '',
            'timestamp': row['timestamp'].isoformat() if row['timestamp'] else None,
            'importance_score': row['importance_score'],
            'importance_reason': row['importance_reason'],
            'output_tokens': row['token_count'],
            'input_tokens': row['input_tokens'],
            'cache_read_tokens': row['cache_read_tokens'],
            'cache_creation_tokens': row['cache_creation_tokens'],
            'semantic_filter_matches': filter_matches.get(msg_id_int, []),
        })

        # Create link from previous message in same session
        if session_id in prev_msg:
            links.append({
                'source': prev_msg[session_id],
                'target': msg_id,
                'session_id': session_id,
                'timestamp': row['timestamp'].isoformat() if row['timestamp'] else None,
            })

        prev_msg[session_id] = msg_id

    return nodes, links


def get_obsidian_notes_with_links() -> list:
    """Scan Obsidian vault for notes with session_id/message_id in frontmatter.

    Returns:
        list of dicts: [{ title, session_id, message_id, created, file_path }, ...]
    """
    # Obsidian vault path - check env var (Docker) then local paths
    vault_path = None

    # Check env var first
    env_vault = os.environ.get("OBSIDIAN_VAULT", "")
    if env_vault and Path(env_vault).exists() and Path(env_vault).is_dir():
        vault_path = Path(env_vault)
    else:
        # Check common paths
        home = Path.home()
        for p in [
            Path("/obsidian-vault"),  # Docker mount
            home / "Documents" / "Obsidian",
            home / "Obsidian",
        ]:
            if p.exists() and p.is_dir():
                vault_path = p
                break

    if not vault_path:
        return []

    notes = []
    frontmatter_pattern = re.compile(r'^---\s*\n(.*?)\n---', re.DOTALL)

    for md_file in vault_path.glob("**/*.md"):
        try:
            content = md_file.read_text(encoding='utf-8')

            # Extract frontmatter
            match = frontmatter_pattern.match(content)
            if not match:
                continue

            frontmatter = match.group(1)

            # Parse session_id and message_id
            session_match = re.search(r'^session_id:\s*(.+)$', frontmatter, re.MULTILINE)
            message_match = re.search(r'^message_id:\s*(\d+)$', frontmatter, re.MULTILINE)
            created_match = re.search(r'^created:\s*(.+)$', frontmatter, re.MULTILINE)

            if session_match:
                note = {
                    'title': md_file.stem,
                    'session_id': session_match.group(1).strip(),
                    'message_id': message_match.group(1).strip() if message_match else None,
                    'created': created_match.group(1).strip() if created_match else None,
                    'file_path': str(md_file),
                }
                notes.append(note)

        except Exception:
            continue

    return notes


def get_topic_graph_data(hours: float = 168) -> tuple[list, list]:
    """Get topic nodes and session-to-topic edges for graph visualization.

    Returns:
        tuple: (topic_nodes, topic_edges) where
            topic_nodes = [{ id, label, session_count }, ...]
            topic_edges = [{ source (session_id), target (topic_id) }, ...]
    """
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    # Get sessions with topics
    cur.execute("""
        SELECT
            s.session_id,
            ss.topics
        FROM claude_sessions.sessions s
        JOIN claude_sessions.session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= %s AND ss.topics IS NOT NULL AND ss.topics != '[]'::jsonb
    """, (since,))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    if not rows:
        return [], []

    topic_nodes = {}  # topic_name -> { id, label, session_count, sessions }
    topic_edges = []

    for row in rows:
        session_id = row['session_id']
        topics = row['topics'] if isinstance(row['topics'], list) else []

        for topic in topics:
            topic_id = f"topic_{topic}"

            if topic_id not in topic_nodes:
                topic_nodes[topic_id] = {
                    'id': topic_id,
                    'label': topic,
                    'session_count': 0,
                    'sessions': []
                }

            topic_nodes[topic_id]['session_count'] += 1
            topic_nodes[topic_id]['sessions'].append(session_id)

            topic_edges.append({
                'source': session_id,
                'target': topic_id,
            })

    return list(topic_nodes.values()), topic_edges


def get_project_session_graph_data(hours: float = 720) -> tuple[list, list]:
    """Get project-based session nodes and edges for hierarchical graph visualization.

    Groups sessions by detected_project (from file paths) when available,
    falls back to primary topic from summary.

    Returns:
        tuple: (nodes, edges) where
            nodes = [{ id, type, label, is_project, session_count, message_count, ... }, ...]
            edges = [{ source, target, edge_type }, ...]
    """
    conn = get_connection()
    cur = conn.cursor()
    since = datetime.now(timezone.utc) - timedelta(hours=hours)

    # Get sessions with metadata, detected_project, and topics
    cur.execute("""
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            COUNT(m.id) as message_count,
            ss.summary,
            ss.topics,
            ss.detected_project
        FROM claude_sessions.sessions s
        LEFT JOIN claude_sessions.messages m ON s.session_id = m.session_id
        LEFT JOIN claude_sessions.session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= %s
        GROUP BY s.session_id, s.cwd, s.start_time, ss.summary, ss.topics, ss.detected_project
        ORDER BY s.start_time DESC
    """, (since,))

    rows = [dict(row) for row in cur.fetchall()]
    cur.close()
    conn.close()

    if not rows:
        return [], []

    # Group sessions by detected_project or primary topic
    groups = {}  # group_name -> { sessions: [], total_messages: 0, is_project: bool }

    for row in rows:
        # Prefer detected_project, fall back to first topic, then 'uncategorized'
        detected = row.get('detected_project')
        session_topics = row['topics'] if isinstance(row['topics'], list) else []

        if detected:
            group_name = detected
            is_project = True
        elif session_topics:
            group_name = session_topics[0]
            is_project = False
        else:
            group_name = 'uncategorized'
            is_project = False

        if group_name not in groups:
            groups[group_name] = {
                'sessions': [],
                'total_messages': 0,
                'all_topics': set(),
                'is_project': is_project,
            }

        groups[group_name]['sessions'].append(row)
        groups[group_name]['total_messages'] += row['message_count'] or 0
        # Track all topics for this group
        for t in session_topics:
            groups[group_name]['all_topics'].add(t)

    # Build nodes and edges
    nodes = []
    edges = []

    for group_name, group_data in groups.items():
        group_id = f"project_{group_name}" if group_data['is_project'] else f"topic_{group_name}"

        # Group node (project or topic)
        nodes.append({
            'id': group_id,
            'type': 'project',
            'label': group_name,
            'is_actual_project': group_data['is_project'],
            'session_count': len(group_data['sessions']),
            'subproject_count': 0,
            'message_count': group_data['total_messages'],
            'related_topics': list(group_data['all_topics'] - {group_name})[:5],
        })

        # Session nodes
        for session in group_data['sessions']:
            session_id = session['session_id']
            session_topics = session['topics'] if isinstance(session['topics'], list) else []

            nodes.append({
                'id': session_id,
                'type': 'session',
                'label': session_id[:8],
                'project': group_name,
                'is_actual_project': group_data['is_project'],
                'subproject': None,
                'topics': session_topics,
                'message_count': session['message_count'] or 0,
                'start_time': session['start_time'].isoformat() if session['start_time'] else None,
                'summary': session['summary'] or 'No summary',
            })

            # Edge: session -> group
            edges.append({
                'source': session_id,
                'target': group_id,
                'edge_type': 'belongs_to',
            })

            # Add edges to secondary topics (lighter weight)
            for secondary_topic in session_topics[1:3]:
                secondary_id = f"topic_{secondary_topic}"
                # Only add if that topic exists as a primary group
                if secondary_topic in groups and not groups[secondary_topic]['is_project']:
                    edges.append({
                        'source': session_id,
                        'target': secondary_id,
                        'edge_type': 'related',
                    })

    return nodes, edges


def format_time_ago(dt) -> str:
    """Format datetime as relative time string."""
    if dt is None:
        return "unknown"

    # Handle pandas Timestamp
    if hasattr(dt, 'to_pydatetime'):
        dt = dt.to_pydatetime()

    # Handle string timestamps
    if isinstance(dt, str):
        try:
            dt = datetime.fromisoformat(dt.replace('Z', '+00:00'))
        except:
            return "unknown"

    now = datetime.now(timezone.utc)

    # Handle timezone-naive datetimes
    if hasattr(dt, 'tzinfo') and dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)

    diff = now - dt

    if diff.total_seconds() < 60:
        return "just now"
    elif diff.total_seconds() < 3600:
        mins = int(diff.total_seconds() / 60)
        return f"{mins}m ago"
    elif diff.total_seconds() < 86400:
        hours = int(diff.total_seconds() / 3600)
        return f"{hours}h ago"
    else:
        days = int(diff.total_seconds() / 86400)
        return f"{days}d ago"
