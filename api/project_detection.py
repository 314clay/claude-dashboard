"""Automated project detection from tool_usages file paths.

Extracts project names from file paths touched during Claude Code sessions.
"""
import json as json_module
import os
import re
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from db.queries import get_connection

# Get home directory dynamically
HOME_DIR = os.environ.get("USER_HOME", str(Path.home()))

# Build project root patterns dynamically based on home directory
# These are common locations where code projects live
def _build_project_roots():
    """Build project root patterns based on current user's home directory."""
    home = HOME_DIR.rstrip('/')

    # Common project directories - customize via PROJECT_DIRS env var
    project_dirs = os.environ.get("PROJECT_DIRS", "w,Documents/GitHub,Documents/github,Projects,code").split(",")

    patterns = []
    for dir_name in project_dirs:
        pattern = rf'{re.escape(home)}/{re.escape(dir_name.strip())}/([^/]+)/'
        patterns.append(pattern)

    return patterns

PROJECT_ROOTS = _build_project_roots()

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


def extract_project_from_path(path: str) -> Optional[str]:
    """Extract project name from a file path.

    Examples:
        ~/projects/myapp/src/main.py -> 'myapp'
        ~/Documents/GitHub/AnkiThings/src/main.ts -> 'AnkiThings'
    """
    if not path:
        return None

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
            SELECT json_extract(tu.tool_input, '$.file_path') as path
            FROM messages m
            JOIN tool_usages tu ON m.id = tu.message_id
            WHERE m.session_id = ?
              AND tu.tool_name IN ('Read', 'Edit', 'Write', 'Glob', 'Grep')
              AND json_extract(tu.tool_input, '$.file_path') IS NOT NULL
        """, (session_id,))

        rows = cur.fetchall()

        if not rows:
            return None

        project_counts = Counter()
        for row in rows:
            project = extract_project_from_path(row['path'])
            if project:
                project_counts[project] += 1

        if not project_counts:
            return None

        return project_counts.most_common(1)[0][0]

    finally:
        cur.close()
        conn.close()


def _build_sqlite_case_expression():
    """Build a SQL CASE expression for project detection using SQLite functions.

    SQLite doesn't have split_part, so we use substr + instr to extract
    the first path component after the project directory.
    """
    home = HOME_DIR.rstrip('/')
    project_dirs = os.environ.get("PROJECT_DIRS", "w,Documents/GitHub,Documents/github,Projects,code").split(",")

    cases = []
    for dir_name in project_dirs:
        dir_path = f"{home}/{dir_name.strip()}"
        like_pattern = f"{dir_path}/%"
        # Extract the part after dir_path/, up to the next /
        # substr(replace(file_path, 'dir_path/', ''), 1, instr(replace(file_path, 'dir_path/', ''), '/') - 1)
        replace_expr = f"replace(file_path, '{dir_path}/', '')"
        extract_expr = f"substr({replace_expr}, 1, instr({replace_expr}, '/') - 1)"
        cases.append(f"""
            WHEN file_path LIKE '{like_pattern}' THEN
                CASE WHEN instr({replace_expr}, '/') > 0
                     THEN {extract_expr}
                     ELSE {replace_expr}
                END""")

    return "CASE" + "".join(cases) + "\n                        ELSE NULL\n                    END"


def detect_all_projects() -> dict[str, str]:
    """Detect projects for all sessions with tool_usages.

    Returns:
        dict mapping session_id -> detected_project
    """
    conn = get_connection()
    cur = conn.cursor()

    case_expr = _build_sqlite_case_expression()
    excluded_list = ", ".join(f"'{name}'" for name in EXCLUDED_NAMES)

    try:
        query = f"""
            WITH file_paths AS (
                SELECT
                    m.session_id,
                    json_extract(tu.tool_input, '$.file_path') as file_path
                FROM messages m
                JOIN tool_usages tu ON m.id = tu.message_id
                WHERE tu.tool_name IN ('Read', 'Edit', 'Write', 'Glob', 'Grep')
                  AND json_extract(tu.tool_input, '$.file_path') IS NOT NULL
                  AND json_extract(tu.tool_input, '$.file_path') NOT LIKE '/var/folders/%'
                  AND json_extract(tu.tool_input, '$.file_path') NOT LIKE '/tmp/%'
            ),
            extracted AS (
                SELECT
                    session_id,
                    {case_expr} as project
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
                  AND project NOT IN ({excluded_list})
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
        """
        cur.execute(query)

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
        cur.execute("""
            SELECT session_id, detected_project
            FROM session_summaries
        """)
        existing = {row['session_id']: row['detected_project'] for row in cur.fetchall()}

        to_update = {}
        for session_id, project in detected.items():
            if session_id not in existing:
                to_update[session_id] = {'project': project, 'action': 'insert'}
            elif existing.get(session_id) is None:
                to_update[session_id] = {'project': project, 'action': 'update'}

        if dry_run:
            return {
                "detected": len(detected),
                "would_update": len([u for u in to_update.values() if u['action'] == 'update']),
                "would_insert": len([u for u in to_update.values() if u['action'] == 'insert']),
                "skipped": len(detected) - len(to_update),
                "projects": dict(Counter(detected.values())),
            }

        updated = 0
        inserted = 0

        for session_id, info in to_update.items():
            if info['action'] == 'update':
                cur.execute("""
                    UPDATE session_summaries
                    SET detected_project = ?
                    WHERE session_id = ?
                """, (info['project'], session_id))
                updated += 1
            else:
                cur.execute("""
                    INSERT INTO session_summaries
                        (session_id, detected_project, generated_at)
                    VALUES (?, ?, ?)
                    ON CONFLICT (session_id) DO UPDATE
                    SET detected_project = excluded.detected_project
                """, (session_id, info['project'], datetime.now(timezone.utc).isoformat()))
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
    print(f"Home directory: {HOME_DIR}")
    print(f"Project roots: {PROJECT_ROOTS}\n")

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
