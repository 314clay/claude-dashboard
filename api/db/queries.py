"""Database query functions for the dashboard (SQLite backend)."""
import os
import re
import sqlite3
import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

# Database path: env var or default to config dir
_default_db_path = os.path.join(
    os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")),
    "dashboard-native",
    "dashboard.db",
)
DB_PATH = os.environ.get("DB_PATH", _default_db_path)

# Home directory for path normalization (configurable via env var)
HOME_DIR = os.environ.get("USER_HOME", str(Path.home()))


def _normalize_path(path: str) -> str:
    """Normalize a path by replacing home directory with ~."""
    if path and HOME_DIR and path.startswith(HOME_DIR):
        return path.replace(HOME_DIR, "~", 1)
    return path


def _ensure_db():
    """Create database directory and initialize schema if needed."""
    db_dir = os.path.dirname(DB_PATH)
    if db_dir:
        os.makedirs(db_dir, exist_ok=True)

    if not os.path.exists(DB_PATH):
        schema_path = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(__file__))), "schema.sqlite.sql")
        conn = sqlite3.connect(DB_PATH)
        if os.path.exists(schema_path):
            with open(schema_path) as f:
                conn.executescript(f.read())
        conn.close()


# Initialize on import
_ensure_db()


def get_connection() -> sqlite3.Connection:
    """Get database connection with Row factory."""
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def _query_to_list(cur, query, params=None) -> list[dict]:
    """Execute query and return list of dicts."""
    cur.execute(query, params or ())
    rows = cur.fetchall()
    return [dict(row) for row in rows]


def get_overview_metrics(hours: float = 24) -> dict:
    """Get high-level metrics for the overview."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    cur.execute("""
        SELECT
            (SELECT COUNT(*) FROM sessions WHERE start_time >= ?) as session_count,
            (SELECT COUNT(*) FROM messages WHERE timestamp >= ?) as message_count,
            (SELECT COUNT(*) FROM messages WHERE timestamp >= ? AND role = 'user') as user_messages,
            (SELECT COUNT(*) FROM messages WHERE timestamp >= ? AND role = 'assistant') as assistant_messages,
            (SELECT COUNT(*) FROM tool_usages WHERE timestamp >= ?) as tool_count
    """, (since, since, since, since, since))

    row = cur.fetchone()
    cur.close()
    conn.close()

    return dict(row)


def get_sessions(hours: float = 24, limit: int = 50) -> list[dict]:
    """Get sessions with message counts."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            s.end_time,
            COUNT(m.id) as total_messages,
            SUM(CASE WHEN m.role = 'user' THEN 1 ELSE 0 END) as user_messages,
            SUM(CASE WHEN m.role = 'assistant' THEN 1 ELSE 0 END) as assistant_messages,
            (strftime('%s', COALESCE(s.end_time, datetime('now'))) - strftime('%s', s.start_time)) / 60.0 as duration_mins
        FROM sessions s
        LEFT JOIN messages m ON s.session_id = m.session_id
        WHERE s.start_time >= ?
        GROUP BY s.session_id, s.cwd, s.start_time, s.end_time
        ORDER BY s.start_time DESC
        LIMIT ?
    """

    rows = _query_to_list(cur, query, (since, limit))
    cur.close()
    conn.close()

    # Clean up paths
    for row in rows:
        row['project'] = _normalize_path(row.get('cwd', ''))
        row['is_active'] = row.get('end_time') is None

    return rows


def get_session_messages(session_id: str) -> list[dict]:
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
        FROM messages m
        WHERE m.session_id = ?
        ORDER BY m.sequence_num
    """

    df = _query_to_list(cur, query, (session_id,))
    cur.close()
    conn.close()
    return df


def get_session_messages_before(session_id: str, before_timestamp: str) -> list[dict]:
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
        FROM messages m
        WHERE m.session_id = ?
          AND m.timestamp <= ?
        ORDER BY m.sequence_num
    """

    df = _query_to_list(cur, query, (session_id, before_timestamp))
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
        FROM session_summaries
        WHERE session_id = ?
    """, (session_id,))

    row = cur.fetchone()
    cur.close()
    conn.close()

    if not row:
        return None

    result = dict(row)
    # Parse JSON topics if stored as string
    if isinstance(result.get('topics'), str):
        try:
            result['topics'] = json.loads(result['topics'])
        except (json.JSONDecodeError, TypeError):
            result['topics'] = []

    return result


def get_session_tools(session_id: str) -> list[dict]:
    """Get tool usages for a specific session."""
    conn = get_connection()
    cur = conn.cursor()

    query = """
        SELECT
            t.tool_name,
            t.tool_input,
            t.timestamp,
            t.sequence_num
        FROM tool_usages t
        JOIN messages m ON t.message_id = m.id
        WHERE m.session_id = ?
        ORDER BY t.timestamp
    """

    df = _query_to_list(cur, query, (session_id,))
    cur.close()
    conn.close()
    return df


def get_tool_usage(hours: float = 24) -> list[dict]:
    """Get tool usage statistics."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            CASE
                WHEN tool_name LIKE 'mcp__%' THEN 'MCP: ' || substr(tool_name, instr(substr(tool_name, 5), '__') + 5)
                ELSE tool_name
            END as tool_category,
            tool_name,
            COUNT(*) as usage_count
        FROM tool_usages
        WHERE timestamp >= ?
        GROUP BY tool_name
        ORDER BY usage_count DESC
    """

    df = _query_to_list(cur, query, (since,))
    cur.close()
    conn.close()
    return df


def get_activity_by_hour(hours: float = 168) -> list[dict]:
    """Get activity breakdown by hour of day (default: past week)."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            CAST(strftime('%H', m.timestamp) AS INTEGER) as hour_of_day,
            CAST(strftime('%w', m.timestamp) AS INTEGER) as day_of_week,
            SUM(CASE WHEN m.role = 'user' THEN 1 ELSE 0 END) as user_messages,
            SUM(CASE WHEN m.role = 'assistant' THEN 1 ELSE 0 END) as assistant_messages,
            COUNT(*) as total_messages
        FROM messages m
        WHERE m.timestamp >= ?
        GROUP BY 1, 2
        ORDER BY 2, 1
    """

    df = _query_to_list(cur, query, (since,))
    cur.close()
    conn.close()
    return df


def get_activity_timeline(hours: float = 24) -> list[dict]:
    """Get activity timeline with hourly buckets."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            strftime('%Y-%m-%d %H:00:00', m.timestamp) as hour_bucket,
            SUM(CASE WHEN m.role = 'user' THEN 1 ELSE 0 END) as user_messages,
            SUM(CASE WHEN m.role = 'assistant' THEN 1 ELSE 0 END) as assistant_messages,
            COUNT(DISTINCT m.session_id) as active_sessions
        FROM messages m
        WHERE m.timestamp >= ?
        GROUP BY 1
        ORDER BY 1
    """

    df = _query_to_list(cur, query, (since,))
    cur.close()
    conn.close()
    return df


def get_projects(hours: float = 168) -> list[dict]:
    """Get project/directory breakdown."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            s.cwd as project,
            COUNT(DISTINCT s.session_id) as session_count,
            COUNT(m.id) as message_count
        FROM sessions s
        LEFT JOIN messages m ON s.session_id = m.session_id
        WHERE s.start_time >= ?
        GROUP BY 1
        ORDER BY message_count DESC
    """

    rows = _query_to_list(cur, query, (since,))
    cur.close()
    conn.close()

    for row in rows:
        row['project'] = _normalize_path(row.get('project', ''))

    return rows


def get_all_messages(hours: float = 24, limit: int = 500) -> list[dict]:
    """Get all messages with session info for data table."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    query = """
        SELECT
            m.id,
            m.session_id,
            m.role,
            m.content,
            m.timestamp,
            m.sequence_num,
            s.cwd
        FROM messages m
        JOIN sessions s ON m.session_id = s.session_id
        WHERE m.timestamp >= ?
        ORDER BY m.timestamp DESC
        LIMIT ?
    """

    rows = _query_to_list(cur, query, (since, limit))
    cur.close()
    conn.close()

    for row in rows:
        row['project'] = _normalize_path(row.get('cwd', ''))
        row['session_short'] = row.get('session_id', '')[:8]

    return rows


def get_recent_user_messages(hours: float = 24, limit: int = 20) -> list[dict]:
    """Get recent user messages for topic overview."""
    conn = get_connection()
    cur = conn.cursor()
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    cur.execute("""
        SELECT
            m.content,
            m.timestamp,
            m.session_id,
            s.cwd
        FROM messages m
        JOIN sessions s ON m.session_id = s.session_id
        WHERE m.role = 'user' AND m.timestamp >= ?
        ORDER BY m.timestamp DESC
        LIMIT ?
    """, (since, limit))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    return [dict(row) for row in rows] if rows else []


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
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

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
            FROM messages m
            JOIN sessions s ON m.session_id = s.session_id
            WHERE m.session_id = ?
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
            FROM messages m
            JOIN sessions s ON m.session_id = s.session_id
            WHERE m.timestamp >= ?
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
            'full_content': content,
            'session_id': session_id,
            'session_short': session_id[:8],
            'project': _normalize_path(row['cwd']) if row['cwd'] else '',
            'timestamp': row['timestamp'],
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
                'timestamp': row['timestamp'],
            })

        prev_msg[session_id] = msg_id

    return nodes, links


def get_obsidian_notes_with_links() -> list:
    """Scan Obsidian vault for notes with session_id/message_id in frontmatter.

    Returns:
        list of dicts: [{ title, session_id, message_id, created, file_path }, ...]
    """
    vault_path = None

    env_vault = os.environ.get("OBSIDIAN_VAULT", "")
    if env_vault and Path(env_vault).exists() and Path(env_vault).is_dir():
        vault_path = Path(env_vault)
    else:
        home = Path.home()
        for p in [
            Path("/obsidian-vault"),
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

            match = frontmatter_pattern.match(content)
            if not match:
                continue

            frontmatter = match.group(1)

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
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    # Get sessions with topics
    cur.execute("""
        SELECT
            s.session_id,
            ss.topics
        FROM sessions s
        JOIN session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= ? AND ss.topics IS NOT NULL AND ss.topics != '[]'
    """, (since,))

    rows = cur.fetchall()
    cur.close()
    conn.close()

    if not rows:
        return [], []

    topic_nodes = {}
    topic_edges = []

    for row in rows:
        session_id = row['session_id']
        # Parse JSON topics
        raw_topics = row['topics']
        if isinstance(raw_topics, str):
            try:
                topics = json.loads(raw_topics)
            except (json.JSONDecodeError, TypeError):
                topics = []
        elif isinstance(raw_topics, list):
            topics = raw_topics
        else:
            topics = []

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


def _parse_topics(raw_topics) -> list:
    """Parse topics from a DB row value (may be JSON string or list)."""
    if isinstance(raw_topics, list):
        return raw_topics
    if isinstance(raw_topics, str):
        try:
            return json.loads(raw_topics)
        except (json.JSONDecodeError, TypeError):
            return []
    return []


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
    since = (datetime.now(timezone.utc) - timedelta(hours=hours)).isoformat()

    cur.execute("""
        SELECT
            s.session_id,
            s.cwd,
            s.start_time,
            COUNT(m.id) as message_count,
            ss.summary,
            ss.topics,
            ss.detected_project
        FROM sessions s
        LEFT JOIN messages m ON s.session_id = m.session_id
        LEFT JOIN session_summaries ss ON s.session_id = ss.session_id
        WHERE s.start_time >= ?
        GROUP BY s.session_id, s.cwd, s.start_time, ss.summary, ss.topics, ss.detected_project
        ORDER BY s.start_time DESC
    """, (since,))

    rows = [dict(row) for row in cur.fetchall()]
    cur.close()
    conn.close()

    if not rows:
        return [], []

    # Group sessions by detected_project or primary topic
    groups = {}

    for row in rows:
        detected = row.get('detected_project')
        session_topics = _parse_topics(row['topics'])

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
        for t in session_topics:
            groups[group_name]['all_topics'].add(t)

    # Build nodes and edges
    nodes = []
    edges = []

    for group_name, group_data in groups.items():
        group_id = f"project_{group_name}" if group_data['is_project'] else f"topic_{group_name}"

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

        for session in group_data['sessions']:
            session_id = session['session_id']
            session_topics = _parse_topics(session['topics'])

            nodes.append({
                'id': session_id,
                'type': 'session',
                'label': session_id[:8],
                'project': group_name,
                'is_actual_project': group_data['is_project'],
                'subproject': None,
                'topics': session_topics,
                'message_count': session['message_count'] or 0,
                'start_time': session['start_time'],
                'summary': session['summary'] or 'No summary',
            })

            edges.append({
                'source': session_id,
                'target': group_id,
                'edge_type': 'belongs_to',
            })

            for secondary_topic in session_topics[1:3]:
                secondary_id = f"topic_{secondary_topic}"
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

    if hasattr(dt, 'to_pydatetime'):
        dt = dt.to_pydatetime()

    if isinstance(dt, str):
        try:
            dt = datetime.fromisoformat(dt.replace('Z', '+00:00'))
        except:
            return "unknown"

    now = datetime.now(timezone.utc)

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
