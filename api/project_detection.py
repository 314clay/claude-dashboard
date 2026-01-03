"""Automated project detection from tool_usages file paths.

Extracts project names from file paths touched during Claude Code sessions.
"""
import re
from collections import Counter
from typing import Optional
import psycopg2
from psycopg2.extras import RealDictCursor


# Known project roots to extract from
PROJECT_ROOTS = [
    r'/Users/clayarnold/w/([^/]+)/',
    r'/Users/clayarnold/Documents/GitHub/([^/]+)/',
    r'/Users/clayarnold/Documents/github/([^/]+)/',
]

# Directories that aren't real projects
EXCLUDED_NAMES = {
    'PATHS.md',
    'docs',
    '.git',
    '.vscode',
    'node_modules',
    '__pycache__',
    '.obsidian',
}


def get_connection():
    """Get database connection."""
    return psycopg2.connect(
        host="localhost",
        port=5433,
        user="clayarnold",
        dbname="connectingservices",
        cursor_factory=RealDictCursor,
    )


def extract_project_from_path(path: str) -> Optional[str]:
    """Extract project name from a file path.

    Examples:
        /Users/clayarnold/w/connect/dashboard/app.py -> 'connect'
        /Users/clayarnold/Documents/GitHub/AnkiThings/src/main.ts -> 'AnkiThings'
    """
    if not path:
        return None

    # Skip temp files
    if path.startswith('/var/folders/') or path.startswith('/tmp/'):
        return None

    for pattern in PROJECT_ROOTS:
        match = re.search(pattern, path)
        if match:
            project = match.group(1)
            if project not in EXCLUDED_NAMES:
                return project

    return None


def detect_project_for_session(session_id: str) -> Optional[str]:
    """Detect the primary project for a session based on file paths.

    Queries tool_usages for file paths and returns the most common project.
    """
    conn = get_connection()
    cur = conn.cursor()

    try:
        cur.execute("""
            SELECT tu.tool_input->>'file_path' as path
            FROM claude_sessions.messages m
            JOIN claude_sessions.tool_usages tu ON m.id = tu.message_id
            WHERE m.session_id = %s
              AND tu.tool_name IN ('Read', 'Edit', 'Write', 'Glob', 'Grep')
              AND tu.tool_input->>'file_path' IS NOT NULL
        """, (session_id,))

        rows = cur.fetchall()

        if not rows:
            return None

        # Count projects
        project_counts = Counter()
        for row in rows:
            project = extract_project_from_path(row['path'])
            if project:
                project_counts[project] += 1

        if not project_counts:
            return None

        # Return most common project
        return project_counts.most_common(1)[0][0]

    finally:
        cur.close()
        conn.close()


def detect_all_projects() -> dict[str, str]:
    """Detect projects for all sessions with tool_usages.

    Returns:
        dict mapping session_id -> detected_project
    """
    conn = get_connection()
    cur = conn.cursor()

    try:
        # Get all file paths grouped by session
        cur.execute("""
            WITH file_paths AS (
                SELECT
                    m.session_id,
                    tu.tool_input->>'file_path' as file_path
                FROM claude_sessions.messages m
                JOIN claude_sessions.tool_usages tu ON m.id = tu.message_id
                WHERE tu.tool_name IN ('Read', 'Edit', 'Write', 'Glob', 'Grep')
                  AND tu.tool_input->>'file_path' IS NOT NULL
                  AND tu.tool_input->>'file_path' NOT LIKE '/var/folders/%'
                  AND tu.tool_input->>'file_path' NOT LIKE '/tmp/%'
            ),
            extracted AS (
                SELECT
                    session_id,
                    CASE
                        WHEN file_path LIKE '/Users/clayarnold/w/%%' THEN
                            split_part(replace(file_path, '/Users/clayarnold/w/', ''), '/', 1)
                        WHEN file_path LIKE '/Users/clayarnold/Documents/GitHub/%%' THEN
                            split_part(replace(file_path, '/Users/clayarnold/Documents/GitHub/', ''), '/', 1)
                        WHEN file_path LIKE '/Users/clayarnold/Documents/github/%%' THEN
                            split_part(replace(file_path, '/Users/clayarnold/Documents/github/', ''), '/', 1)
                        ELSE NULL
                    END as project
                FROM file_paths
            ),
            project_counts AS (
                SELECT
                    session_id,
                    project,
                    COUNT(*) as cnt
                FROM extracted
                WHERE project IS NOT NULL
                  AND project != ''
                  AND project NOT IN ('PATHS.md', 'docs', '.git', '.vscode', 'node_modules')
                GROUP BY session_id, project
            ),
            ranked AS (
                SELECT
                    session_id,
                    project,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY cnt DESC) as rn
                FROM project_counts
            )
            SELECT session_id, project
            FROM ranked
            WHERE rn = 1
        """)

        return {row['session_id']: row['project'] for row in cur.fetchall()}

    finally:
        cur.close()
        conn.close()


def backfill_detected_projects(dry_run: bool = False) -> dict:
    """Backfill detected_project for all sessions.

    Args:
        dry_run: If True, don't actually update, just return what would change.

    Returns:
        dict with stats about the backfill operation.
    """
    detected = detect_all_projects()

    if not detected:
        return {"detected": 0, "updated": 0, "skipped": 0}

    conn = get_connection()
    cur = conn.cursor()

    try:
        # Get existing detected_projects
        cur.execute("""
            SELECT session_id, detected_project
            FROM claude_sessions.session_summaries
        """)
        existing = {row['session_id']: row['detected_project'] for row in cur.fetchall()}

        # Find sessions needing update
        to_update = {}
        for session_id, project in detected.items():
            if session_id not in existing:
                # No summary exists - need to create one
                to_update[session_id] = {'project': project, 'action': 'insert'}
            elif existing.get(session_id) is None:
                # Summary exists but no project detected yet
                to_update[session_id] = {'project': project, 'action': 'update'}
            # Skip if already has a detected_project (don't overwrite manual assignments)

        if dry_run:
            return {
                "detected": len(detected),
                "would_update": len([u for u in to_update.values() if u['action'] == 'update']),
                "would_insert": len([u for u in to_update.values() if u['action'] == 'insert']),
                "skipped": len(detected) - len(to_update),
                "projects": dict(Counter(detected.values())),
            }

        # Perform updates
        updated = 0
        inserted = 0

        for session_id, info in to_update.items():
            if info['action'] == 'update':
                cur.execute("""
                    UPDATE claude_sessions.session_summaries
                    SET detected_project = %s
                    WHERE session_id = %s
                """, (info['project'], session_id))
                updated += 1
            else:
                # Insert minimal summary with just the detected project
                cur.execute("""
                    INSERT INTO claude_sessions.session_summaries
                        (session_id, detected_project, generated_at)
                    VALUES (%s, %s, NOW())
                    ON CONFLICT (session_id) DO UPDATE
                    SET detected_project = EXCLUDED.detected_project
                """, (session_id, info['project']))
                inserted += 1

        conn.commit()

        return {
            "detected": len(detected),
            "updated": updated,
            "inserted": inserted,
            "skipped": len(detected) - len(to_update),
            "projects": dict(Counter(detected.values())),
        }

    finally:
        cur.close()
        conn.close()


def get_project_summary() -> list[dict]:
    """Get summary of detected projects with session counts.

    Returns list sorted by session count descending.
    """
    detected = detect_all_projects()
    project_counts = Counter(detected.values())

    return [
        {"project": project, "session_count": count}
        for project, count in project_counts.most_common()
    ]


if __name__ == "__main__":
    import json

    print("Detecting projects from file paths...\n")

    # Dry run first
    result = backfill_detected_projects(dry_run=True)
    print("Dry run results:")
    print(json.dumps(result, indent=2))

    print("\n" + "="*50)
    response = input("\nProceed with backfill? [y/N]: ")

    if response.lower() == 'y':
        result = backfill_detected_projects(dry_run=False)
        print("\nBackfill complete:")
        print(json.dumps(result, indent=2))
    else:
        print("Aborted.")
