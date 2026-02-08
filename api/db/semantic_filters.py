"""Database query functions for semantic filters (SQLite backend)."""
from datetime import datetime, timezone
from .queries import get_connection


def get_all_filters() -> list[dict]:
    """Get all semantic filters with stats (total_scored, matches count).

    Returns:
        list of dicts: [{ id, name, query_text, created_at, is_active, total_scored, matches }, ...]
    """
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT
            f.id,
            f.name,
            f.query_text,
            f.created_at,
            f.is_active,
            COUNT(r.id) as total_scored,
            SUM(CASE WHEN r.matches = 1 THEN 1 ELSE 0 END) as matches
        FROM semantic_filters f
        LEFT JOIN semantic_filter_results r ON f.id = r.filter_id
        GROUP BY f.id, f.name, f.query_text, f.created_at, f.is_active
        ORDER BY f.created_at DESC
    """)

    rows = cur.fetchall()
    cur.close()
    conn.close()

    filters = [dict(row) for row in rows]
    for f in filters:
        f['is_active'] = bool(f.get('is_active'))
    return filters


def create_filter(name: str, query_text: str) -> dict:
    """Create a new semantic filter.

    Args:
        name: Unique name for the filter
        query_text: The semantic query text

    Returns:
        dict with created filter data: { id, name, query_text, created_at, is_active }

    Raises:
        Exception if name is not unique
    """
    conn = get_connection()
    cur = conn.cursor()

    now = datetime.now(timezone.utc).isoformat()
    cur.execute("""
        INSERT INTO semantic_filters (name, query_text, created_at, is_active)
        VALUES (?, ?, ?, 1)
    """, (name, query_text, now))

    filter_id = cur.lastrowid
    conn.commit()

    # Fetch the created row
    cur.execute("""
        SELECT id, name, query_text, created_at, is_active
        FROM semantic_filters WHERE id = ?
    """, (filter_id,))

    row = cur.fetchone()
    cur.close()
    conn.close()

    result = dict(row)
    result['is_active'] = bool(result.get('is_active'))
    return result


def delete_filter(filter_id: int) -> bool:
    """Delete a semantic filter and its results.

    Args:
        filter_id: The filter ID to delete

    Returns:
        True if deleted, False if filter not found
    """
    conn = get_connection()
    cur = conn.cursor()

    # Delete results first (foreign key constraint)
    cur.execute("""
        DELETE FROM semantic_filter_results
        WHERE filter_id = ?
    """, (filter_id,))

    # Delete the filter
    cur.execute("""
        DELETE FROM semantic_filters
        WHERE id = ?
    """, (filter_id,))

    deleted = cur.rowcount > 0
    conn.commit()
    cur.close()
    conn.close()

    return deleted


def get_filter_status(filter_id: int) -> dict | None:
    """Get scoring progress for a specific filter.

    Args:
        filter_id: The filter ID

    Returns:
        dict: { filter_id, name, total, scored, pending, matches }
        or None if filter not found
    """
    conn = get_connection()
    cur = conn.cursor()

    # Get filter info
    cur.execute("""
        SELECT id, name, query_text, is_active
        FROM semantic_filters
        WHERE id = ?
    """, (filter_id,))

    filter_row = cur.fetchone()
    if not filter_row:
        cur.close()
        conn.close()
        return None

    # Get total message count
    cur.execute("""
        SELECT COUNT(*) as total
        FROM messages
    """)
    total = cur.fetchone()['total']

    # Get scored and matches counts for this filter
    cur.execute("""
        SELECT
            COUNT(*) as scored,
            SUM(CASE WHEN matches = 1 THEN 1 ELSE 0 END) as matches
        FROM semantic_filter_results
        WHERE filter_id = ?
    """, (filter_id,))

    result_row = cur.fetchone()
    scored = result_row['scored']
    matches = result_row['matches'] or 0

    cur.close()
    conn.close()

    return {
        'filter_id': filter_row['id'],
        'name': filter_row['name'],
        'query_text': filter_row['query_text'],
        'is_active': filter_row['is_active'],
        'total': total,
        'scored': scored,
        'pending': total - scored,
        'matches': matches,
    }


def get_message_filter_matches(message_ids: list[int]) -> dict[int, list[int]]:
    """Get filter matches for a list of message IDs.

    Args:
        message_ids: List of message IDs to check

    Returns:
        dict mapping message_id -> list of filter_ids that match
    """
    if not message_ids:
        return {}

    conn = get_connection()
    cur = conn.cursor()

    # SQLite has a variable limit (~999), batch large queries
    rows = []
    batch_size = 900
    for i in range(0, len(message_ids), batch_size):
        batch = message_ids[i:i + batch_size]
        placeholders = ','.join('?' * len(batch))
        cur.execute(f"""
            SELECT
                r.message_id,
                r.filter_id
            FROM semantic_filter_results r
            JOIN semantic_filters f ON r.filter_id = f.id
            WHERE r.message_id IN ({placeholders})
              AND r.matches = 1
              AND f.is_active = 1
        """, batch)
        rows.extend(cur.fetchall())

    cur.close()
    conn.close()

    # Build message_id -> [filter_ids] mapping
    result = {}
    for row in rows:
        msg_id = row['message_id']
        filter_id = row['filter_id']
        if msg_id not in result:
            result[msg_id] = []
        result[msg_id].append(filter_id)

    return result
